# Lab4 2023010745 赵睿 计32

## 实现功能

### 维护前几次实验的功能

除按实验要求删除sys_trace外，其余前几次实验的syscall（sys_get_time、sys_mmap、sys_munmap、sys_spawn、sys_set_priority）以及stride调度算法都从ch5搬到ch6上。冲突主要有两处：os/src/syscall/process.rs的use块需要合并ch6对easy-fs的依赖和ch5对loader的依赖，并顺手把sys_spawn从loader::get_app_data_by_name改为按ch6风格通过open_file加载ELF；ch6的TaskControlBlockInner比ch5多一个fd_table字段，spawn构造新TCB时按new的做法补一份只含stdin/stdout/stderr的fd_table。

### 实现sys_fstat

sys_fstat的syscall id为80，从打开的文件描述符获取inode号、文件类型和硬链接数。

在File trait里加一个默认返回None的fstat方法。OSInode重载它，从内部的easy-fs Inode读出inode号和是否目录，再通过ROOT_INODE统计指向该inode号的目录项数得到nlink。其它实现了File trait的类型（Stdin、Stdout）不override，自然返回None，sys_fstat直接判定为无效fd。

syscall处理函数从当前进程的fd_table取出文件，调用File::fstat拿到Stat结构体，再用translated_byte_buffer把它分段拷到用户态地址，照顾Stat可能跨页的情况。

### 实现sys_linkat、sys_unlinkat

sys_linkat的syscall id为37、sys_unlinkat为35。两个syscall的参数列表里都有dirfd和flags，但当前ch6的dispatcher只把path参数透传到内核：

```rust
SYSCALL_LINKAT => sys_linkat(args[1] as *const u8, args[3] as *const u8),
SYSCALL_UNLINKAT => sys_unlinkat(args[1] as *const u8),
```

因此内核侧的两个函数只需要处理路径字符串，dirfd和flags按AT_FDCWD与0的语义被用户库默认填好。

在easy-fs/src/vfs.rs上为Inode新增四个方法支撑硬链接和fstat：

- inode_id：从block_id和block_offset反算inode号，需要EasyFileSystem额外暴露一个get_inode_id方法
- is_dir：读出DiskInode的type_判断目录或文件，给StatMode区分DIR/FILE使用
- link(old_name, new_name)：在该目录中查找old_name，找到后追加一个指向同一inode号的新目录项
- unlink(name)：在该目录中找到name对应的目录项并将其整体清零；同一次modify_disk_inode调用里就把"查找目标"、"统计目标inode剩余的硬链接数"、"清零目标槽"三件事一并做完，避免重复获取目录块的cache锁；若链接计数归零则调用clear_size释放数据块、再调用新增的EasyFileSystem::dealloc_inode释放inode位图比特

另有一个link_count方法供sys_fstat取Stat.nlink：扫描目录中指向给定inode号的非空条目数。两处扫描都跳过name为空的目录项——这正是unlink遗留的空槽，保证它们既不被算入nlink也不影响find/ls的查找正确性。

需要注意的是，原版write_at在每次写完后无条件调用block_cache_sync_all，这在ch6_file3那种"50次小写入×10轮"的压力测试下会触发大量虚拟块设备写。BlockCache本身在LRU淘汰时已经通过Drop自动sync脏块，create/link/unlink/clear这些提交点也各自调用了sync_all，所以write_at里的逐次sync是冗余的，去掉后压力测试的吞吐显著上升。

在os/src/fs/mod.rs给Stat加一个new构造函数以减少Stat字段的重复填充。os/src/fs/inode.rs封装两个对ROOT_INODE的薄函数link_at、unlink_at，再供sys_linkat、sys_unlinkat直接调用，syscall层不直接持有ROOT_INODE。

linkat语义上要求拒绝同名链接，所以sys_linkat里对old_name == new_name先做一次检查直接返回-1；Inode::link内部也会校验new_name是否已经存在，存在则返回None。unlinkat查不到name时返回-1。

## 简答作业

### 在我们的easy-fs中，root inode起着什么作用？如果root inode中的内容损坏了，会发生什么？

root inode是easy-fs中inode号固定为0的那一个inode，由EasyFileSystem::root_inode暴露出来。它作为整个文件系统唯一的根目录：所有用户文件以及sys_linkat创建的硬链接，对应的目录项都直接平铺在root inode管理的目录数据里，不存在子目录的概念。

root inode具体扮演三种角色：

- 文件查找的入口。open_file调用ROOT_INODE.find遍历root inode的目录项数据，按文件名匹配出对应的inode号，再由efs用inode号定位到磁盘inode。
- 文件创建/链接/解除链接的目标目录。create往root inode的目录数据末尾追加一个新目录项；link同样在root inode下新增目录项；unlink在root inode的目录数据中找到匹配项并置空。
- nlink统计的依据。sys_fstat在统计硬链接数时遍历root inode下所有目录项，计数指向目标inode号的非空条目。

如果root inode的内容损坏，按损坏的部分不同会出现不同后果：

- 若root inode本身（在inode区的那个DiskInode）被破坏，例如direct/indirect指针被覆盖：硬件按错误的块号去读目录数据，整个根目录变得无法解析。OSInode首次访问时就会失败，open_file、link、unlink、ls全部不可用，等于整个文件系统对用户不可见，已写入的用户文件虽然在磁盘上还在，但找不到入口。
- 若DiskInode未损坏但其指向的数据块（即目录项数组）部分损坏：会观察到部分文件名乱码、inode号指向错乱的目录项。find可能误把请求文件A路由到另一个inode B的数据上，造成读到错文件甚至跨文件写入污染；link_count统计也会偏差，造成sys_fstat返回的nlink不可信，进而unlink可能在错误的时刻释放尚有其它链接的inode，引发数据块被错误回收。
- 若root inode的size字段错乱：循环上界`(size as usize) / DIRENT_SZ`错位，可能少扫到合法目录项（文件查不到、表现为"明明create过却找不到"），或多扫到尾部未初始化的内存（命中乱掉的DirEntry，触发assertion失败甚至非法inode号操作）。

由于easy-fs只有一级目录、所有元信息都集中在root inode上，它的损坏代价基本等同于整个文件系统失效——没有任何冗余目录或fsck机制能从其它路径重新建立映射。

### 举出使用pipe的一个实际应用的例子

shell管道是pipe最常见的实际应用。比如用cat配合wc统计某个文件的行数：

```
cat /var/log/syslog | wc -l
```

shell的实现细节：父shell先pipe创建一对管道（读端fd[0]+写端fd[1]）；fork出子进程A，把stdout dup2到fd[1]，关掉两个原始fd，exec("cat")；再fork子进程B，把stdin dup2到fd[0]，关掉两个原始fd，exec("wc -l")；父shell关掉自己的fd[0]和fd[1]，wait两个子进程。整个过程中cat的stdout和wc的stdin被pipe在内核里直接拼起来，两个进程不用知道彼此的pid或预先约定文件，cat退出时写端引用计数归零，wc下一次read返回0感知到EOF自然终止。

类似的常用组合还有ps aux | grep nginx（找特定名字的进程）、dmesg | tail -n 50（看最近的内核日志）等。共同特征是把小而专一的工具用管道串成流水线，每个工具只关心自己的stdin/stdout，组合后能完成相当复杂的任务。

### 多进程通信机制设计

两两建立管道的麻烦在于：N个进程互相通信需要$N(N-1)/2$条管道，每条还要双方都拿到fd（靠fork继承或fd传递），开销大。

设计一个**邮箱**机制：内核维护一张全局邮箱表，每个邮箱由字符串name标识、内部是一个有界FIFO队列。任何进程只要知道name就能往邮箱投递或取走消息，不需要预先和别人握手。

**系统调用接口**：

```rust
// 创建/打开name邮箱：不存在则按capacity创建，已存在则附加引用；返回fd
fn sys_mbox_open(name: *const u8, capacity: usize) -> isize;

// 向name邮箱投递一条消息。队列满时按flags决定阻塞或返回-1
fn sys_mbox_send(name: *const u8, buf: *const u8, len: usize, flags: u32) -> isize;

// 从fd对应邮箱取一条消息。队列空时按flags阻塞或返回-1；返回实际拷贝字节数
fn sys_mbox_recv(fd: usize, buf: *mut u8, len: usize, flags: u32) -> isize;

// 关闭fd对邮箱的引用；引用归零时邮箱整体回收
fn sys_mbox_close(fd: usize) -> isize;
```

flags可包含NONBLOCK（非阻塞）、BROADCAST（广播给所有当前订阅者，相当于pub-sub的topic）。

**内核数据结构**：

```rust
struct Mailbox {
    name: String,
    capacity: usize,
    queue: VecDeque<Message>,   // 消息环形队列
    senders: WaitQueue,          // 满时阻塞的发送者
    receivers: WaitQueue,        // 空时阻塞的接收者
    ref_count: usize,            // 当前打开它的fd数
}

struct Message {
    sender_pid: usize,           // 发件人pid，便于回信
    payload: Vec<u8>,            // 消息体
}

static MAILBOX_TABLE: BTreeMap<String, Arc<Mutex<Mailbox>>>;
```

**通信模式覆盖**：

- 点对点：双方约定name="ch_AB"，A发B收，无需fork继承fd。
- 多生产者单消费者：name="log_sink"，多个生产者并发send，单个消费者循环recv。
- 多消费者负载均衡：name="task_queue"，每条消息只交付给某一个recv者（先到先得）。
- 广播：send带BROADCAST标志时，把消息拷贝到所有当前订阅者的私有队列里。

**相对pair-pipe的优势**：

| 维度 | 两两pipe | 命名邮箱 |
|:---|:---|:---|
| 建立通信 | 必须先pipe+fork+dup2/SCM_RIGHTS传fd | 知道name即可open，进程间完全解耦 |
| N方通信成本 | $O(N^2)$条管道+fd管理 | $O(1)$（共享一个name） |
| 通信模式 | 单向流，一对一 | 一对一/多对一/一对多/广播都支持 |
| 消息边界 | 字节流，应用自己分包 | 内核保留消息边界，每次recv取一条完整message |
| 跨家族进程 | 难，需要传fd | 任意进程只要知道name即可参与 |
| 发送方身份 | 不携带 | Message里带sender_pid，天然支持回信 |

## 荣誉准则

在完成本次实验的过程（含此前学习的过程）中，我曾分别与 **以下各位** 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

无

此外，我也参考了 **以下资料** ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

无

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按"-100"分计。

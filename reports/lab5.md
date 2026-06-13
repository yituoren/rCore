# Lab5 2023010745 赵睿 计32

## 实现功能

### 维护前几次实验的功能

ch8框架相对ch6改动很大：把ch6的TaskControlBlock拆成ProcessControlBlock+TaskControlBlock，地址空间、fd_table、子进程列表都搬到PCB上，原来的stride/priority字段也不见了；syscall入口的trace、current_task路径也跟着重写。按照实验指导书"frame change is significant; merging prior content not required except sys_get_time"的提示，ch4/ch5的mmap/munmap/spawn/set_priority和stride调度本轮不再维护，只补回ch4的sys_get_time与ch6的easy-fs改动。

通过cherry-pick把ch6的feat: ch6、fix: unlink、fix: sync三个commit挪到ch8。冲突主要在os/src/fs/、os/src/syscall/fs.rs上：

- easy-fs/src/efs.rs、easy-fs/src/vfs.rs、easy-fs/src/lib.rs、os/src/fs/mod.rs/inode.rs可以基本干净并入，得到link/unlink/inode_id/link_count、Inode::Drop自动sync、Stat::new辅助构造、File trait的fstat默认方法等。
- os/src/syscall/fs.rs里sys_fstat、sys_linkat、sys_unlinkat的trace信息要改成ch8风格`current_task().unwrap().process.upgrade().unwrap().getpid()`；fd_table从task里挪到了process里，所以查fd_table要走current_process().inner_exclusive_access().fd_table。

sys_get_time直接照搬ch4的实现：用translated_byte_buffer把用户态TimeVal指针翻成若干物理切片，按段拷贝，跨页也安全。

### 实现sys_enable_deadlock_detect

sys_enable_deadlock_detect的syscall id为469，参数1开启检测、0关闭，非法值返回-1。功能上只是把当前进程inner里的一个deadlock_detect开关置位：

```rust
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    if enabled > 1 { return -1; }
    let process = current_process();
    process.inner_exclusive_access().deadlock_detect = enabled == 1;
    0
}
```

### 死锁检测：银行家算法

死锁检测要按进程隔离，且mutex和semaphore互不串扰，按指导书的提示分两套独立的银行家结构维护。

在ProcessControlBlockInner里新增七个字段：

```rust
pub deadlock_detect: bool,
pub mutex_available: Vec<i32>,        // 每个mutex的可用单元数（0或1）
pub mutex_allocation: Vec<Vec<i32>>,  // [tid][mid] 持有数
pub mutex_need: Vec<Vec<i32>>,        // [tid][mid] 申请中数
pub sem_count: Vec<i32>,              // 同步semaphore.count（可为负，表示有等待者）
pub sem_allocation: Vec<Vec<i32>>,    // [tid][sid] 已成功down的次数 - 已up的次数
pub sem_need: Vec<Vec<i32>>,          // [tid][sid] 正在阻塞中的down请求
```

抽出一个os/src/sync/deadlock.rs放算法主体：is_safe就是教材里的银行家伪代码，加上ensure_vec/ensure_matrix两个辅助函数按需扩容（线程id和资源id是逐步分配的，矩阵不能一开始就开满）。

```rust
pub fn is_safe(available: &[i32], allocation: &[Vec<i32>], need: &[Vec<i32>]) -> bool {
    let n_thread = allocation.len();
    let n_res = available.len();
    let mut work: Vec<i32> = available.to_vec();
    let mut finish = vec![false; n_thread];
    loop {
        let mut progressed = false;
        for i in 0..n_thread {
            if finish[i] { continue; }
            let row_need = &need[i];
            let can = (0..n_res).all(|j| row_need.get(j).copied().unwrap_or(0) <= work[j]);
            if can {
                finish[i] = true;
                let row_alloc = &allocation[i];
                for j in 0..n_res { work[j] += row_alloc.get(j).copied().unwrap_or(0); }
                progressed = true;
            }
        }
        if !progressed { break; }
    }
    finish.iter().all(|&f| f)
}
```

mutex路径（sys_mutex_lock）：拿到process inner后先按mutex_list扩容三个矩阵；若开了deadlock_detect，先把need[tid][mid] += 1，跑is_safe，若不安全就回滚need并返回-0xdead；通过后drop inner，调用真正的mutex.lock()阻塞等锁；锁拿到后再回来更新allocation和available。sys_mutex_unlock对称地把allocation -1、available +1。sys_mutex_create新增一个mutex时在mutex_available尾部追加1。

semaphore路径（sys_semaphore_down）思路一致，但因为semaphore是计数信号量，available要从sem_count里临时计算：可用单元 = max(0, count)。当count为负时表示有等待者，没有空闲单元，但银行家算法只看available里的非负部分即可。检查通过后调用sem.down()阻塞，回来更新sem_count -= 1、allocation += 1。sys_semaphore_up对称地allocation -= 1（如果该线程之前down过）、sem_count += 1。sys_semaphore_create把sem_count[sid]初始化为res_count。

需要注意的几个点：

- 银行家检查必须在调用底层mutex.lock()/sem.down()之前完成，否则一旦真的阻塞下去就晚了；
- 检查时need[tid][res]要"暂时+1"以反映"假如我把这个请求纳入系统"，不安全就回滚；
- 安全后才放行真实的lock/down，等其返回再把need-1、allocation+1。
- mutex_available、sem_count要在每次创建新资源时同步初始化，不然银行家矩阵的列数和资源数会对不上。

## 简答作业

### 1. 主线程退出时各类资源的回收与TaskControlBlock引用

当主线程退出（进而触发整个进程退出）时，需要回收的资源大致分三层：

**进程持有的"非线程"资源**（在ProcessControlBlockInner里）：

- memory_set：进程的整个用户地址空间，包括ELF段映射、用户栈、TrapContext页、堆等。这些Framed类型的MapArea必须释放，对应的物理页帧才能归还给frame allocator。memory_set随PCB drop时自动释放。
- fd_table：所有打开的文件/管道/socket的Arc引用。drop fd_table会把每个Arc递减，OSInode、Pipe等如果引用归零会执行各自的Drop（OSInode的Drop会触发block_cache_sync_all把脏块落盘），关键资源不会泄漏。
- mutex_list、semaphore_list、condvar_list：这些同步原语本身只是Arc<dyn Mutex>等智能指针，drop就回收。
- task_res_allocator：tid位图。

**主线程及其它线程持有的per-thread资源**（在TaskControlBlockInner.res里）：

- TaskUserRes包含tid、TrapContext所在物理页号、用户栈的虚拟范围。res的Drop里要释放tid（task_res_allocator.dealloc）、删除地址空间中的TrapContext页和用户栈MapArea，否则那些物理页帧不会归还给frame allocator。
- KernelStack：每个线程独立的内核栈，KernelStack::Drop会把内核地址空间里对应的MapArea删除并释放物理页帧。

**TaskControlBlock的引用**散落在几处，必须一并收回，否则TCB的Arc引用计数不归零，TCB自身的Drop就不会触发，res和kstack也都释放不掉：

- ProcessControlBlockInner.tasks：每个线程都在它所属进程的tasks Vec里被Arc持有。进程退出时遍历tasks并Option::take()掉，把这一份强引用释放。
- 任务调度器TASK_MANAGER的ready_queue：处于Ready状态的线程在这里。退出路径要确保它们要么被take出来释放，要么因为状态转Zombie后调度器扫描时跳过/丢弃。
- 各种wait_queue：semaphore/mutex/condvar的等待队列里也持有Arc<TaskControlBlock>。如果有线程恰好阻塞在某个同步原语上，那条引用必须在进程清理时一并drop。一种省事的做法是在exit_current_and_run_next里把PCB里所有sync原语显式drop，从而让它们的wait_queue一并drop。
- Processor.current：CPU当前在跑的那个线程TCB也是Arc。退出时由调度路径自然take()。
- 定时器add_timer的列表：sys_sleep会把TCB放进定时器队列。线程退出时如果它还在timer里，要清理；或者timer到时唤醒时发现线程已是Zombie就丢弃。

只要上面这些位置都松开了Arc引用，TCB的strong_count就能归零，TCB::Drop触发，inner里的res和kstack依次drop，物理页和tid都得以回收。如果遗漏其中任何一个引用点，主线程退出后那个TCB会一直作为"幽灵"留在内核里，对应的物理页帧、内核栈、tid都泄漏，长期跑下来会逐步耗尽资源。

### 2. 两种Mutex实现的差别

对比的两种实现是MutexSpin和MutexBlocking。MutexSpin的lock()在锁被占用时不让出CPU，而是循环自旋——具体到rCore是在循环里调用suspend_current_and_run_next()主动yield，等下次被调度回来再检查锁状态。MutexBlocking则在拿不到锁时把当前线程放进自己的wait_queue并block_current_and_run_next，等unlock时由持锁者pop_front唤醒。

主要差别：

**调度行为**：

- 自旋mutex在等待期间反复进入"runnable→running→runnable"循环。即使每轮都立刻yield，从调度器视角看它仍然是个就绪进程，会被排队调度一次再立刻让出，浪费了一次context switch的成本。
- 阻塞mutex进入wait_queue后状态变Blocked，调度器看不见它，直到unlock显式唤醒。等待期间完全不占调度器配额。

**唤醒及时性**：

- 自旋mutex谁先被调度到、谁先检查锁、谁就先拿到锁——没有顺序保证，可能造成"饿一个、热一个"的不公平。极端场景下持锁者刚unlock时不能立即把锁交给"最早申请的等待者"，谁碰巧此刻被调度就吃到。
- 阻塞mutex的wait_queue是FIFO的，unlock时pop_front出来唤醒，保证了申请顺序的公平性。

**功能问题**：

- 自旋mutex如果在单核且未启用时钟中断抢占的情况下被一个长时间持锁的线程占住，等待者就算让出CPU也只会轮流过来再让出，整体没法推进——本质上变成"忙等持锁线程下次被调度并unlock"。在ch8这种支持时钟抢占的环境下还能工作，但CPU利用率明显比阻塞版差。
- 阻塞mutex一旦实现错误（比如unlock时忘了把队首线程唤醒），等待者会一直Blocked，这是一种永久挂起，比自旋的"白白消耗调度时间"更难诊断。
- 阻塞mutex要正确处理"unlock时wait_queue为空vs非空"两种情况：空就把locked置回false，非空就直接把锁的所有权移交给被唤醒的线程而**不要**把locked清零，否则下一次lock的人可能插队抢到锁，造成被唤醒线程拿不到锁还得再睡一次。
- 死锁检测：自旋mutex由于等待者频繁回到调度器，可以多次进入sys_mutex_lock做银行家检查；阻塞mutex一旦进了wait_queue就不再走syscall入口了——我们的实现里安全性检查必须发生在调用mutex.lock()之前，所以两种mutex在死锁检测上需要的入口逻辑实际上是一致的，关键在于"在阻塞之前"完成检查。

总结一句：自旋适合临界区极短、对响应延迟敏感、不可调度的场景（例如RTOS或Linux的spinlock_irqsave）；阻塞适合临界区可能较长、上下文切换开销可接受的场景（绝大多数应用层mutex）。两者用错地方都会出问题——把spin用到长临界区上CPU空转，把blocking用到中断上下文里会因为不能调度而死掉。

## 荣誉准则

在完成本次实验的过程（含此前学习的过程）中，我曾分别与 **以下各位** 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

无

此外，我也参考了 **以下资料** ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

无

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按"-100"分计。

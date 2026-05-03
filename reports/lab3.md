# Lab3 2023010745 赵睿 计32

## 实现功能

### 移植sys_get_time、sys_mmap、sys_munmap

ch5引入进程抽象后，每个进程拥有独立的页表和地址空间，原本以全局TaskManager为入口的系统调用需要改写为通过current_task取得当前进程的TCB再访问其memory_set。sys_trace在ch5实验要求中已删除，对应代码一并去除。

sys_get_time沿用ch4的思路：用translated_byte_buffer将用户态TimeVal指针翻译为若干物理切片，按段拷贝时间数据，自动处理跨页边界的情形。current_user_token从当前任务的memory_set token取得。

sys_mmap和sys_munmap的核心逻辑直接搬到ch5的MemorySet上：syscall入口校验start页对齐和port参数（低3位非零、其它位为0），然后调用当前任务memory_set的mmap方法完成地址区间检查、Framed类型MapArea的创建和插入；munmap还要求start+len页对齐，遍历范围内每一页确认已映射后逐个unmap，最后用retain清掉空的MapArea。

### 实现sys_spawn

sys_spawn的syscall id为400，语义介于fork和exec之间：直接以ELF文件为参数创建一个新进程并加入就绪队列，但不复制父进程的地址空间。在TaskControlBlock上新增spawn方法，过程类似new：解析ELF得到全新的memory_set、user_sp和entry_point，分配PID和内核栈，初始化TrapContext、base_size、heap_bottom等字段，stride字段初始化为0、priority为16；与new的差别在于spawn需要同时建立父子关系——把self的弱引用写入子的parent字段，并把子的Arc追加进父的children数组。

syscall处理函数按以下流程：用translated_str从用户态读取路径字符串，调用get_app_data_by_name查找ELF；找不到时返回-1（拒绝非法文件名）；找到则调用current_task的spawn得到子TCB，add_task入就绪队列，最后返回子pid。

### 实现stride调度

按实验要求采用stride调度算法，在TaskControlBlockInner中新增两个字段：

- stride，累积步长，初始为0
- priority，优先级，初始为16

三个TCB构造路径（new、fork、spawn）都把stride初始化为0、priority初始化为16。

调度策略由TaskManager的fetch方法实现：遍历ready_queue找出stride最小的任务，从队列中移除并返回；选中任务的stride在出队的同时累加BIG_STRIDE / priority。BIG_STRIDE取常量16777216 = 2^24，远大于任何合理的priority值，足以减小整除取整误差，又远小于usize范围，让有符号回绕比较仍然有效。

stride比较采用有符号差值法处理回绕：

```rust
if (s as isize - min_stride as isize) < 0 {
    min_idx = i;
    min_stride = s;
}
```

新增sys_set_priority（syscall id 140）：检查prio >= 2，否则直接返回-1；合法时写入当前任务的priority字段并返回prio本身。

## 简答作业

### 1. p1.stride=255、p2.stride=250、pass=10、8位无符号下p2执行后是否轮到p1

不会轮到p1。

p2当前stride为250，被调度执行后p2.stride累加pass=10，本应得到260；但stride以8位无符号整数存储，260会被截断回4。此时p1.stride=255、p2.stride=4，按朴素的无符号大小比较p2.stride < p1.stride，调度器仍会判定p2拥有更小的stride，再次选中p2，p1被无限期跳过。

根本原因是stride的累加发生了回绕，朴素比较把"刚刚走得更远的p2"误判为"走得更少"，破坏了stride算法依赖的"小stride=更早被选"这一不变量。修复方式有两种：要么使比较回绕安全——用有符号差值代替直接比较（见第3题）；要么扩大stride的位宽，使其在合理时间内不会回绕。

### 2. 为什么priority≥2能保证STRIDE_MAX-STRIDE_MIN≤BigStride/2

每次调度选出stride最小的进程运行，运行后该进程的stride累加pass = BigStride / priority。由于priority ≥ 2，因此pass ≤ BigStride / 2。

考虑任意时刻就绪队列里所有进程的stride集合，记最大值为STRIDE_MAX、最小值为STRIDE_MIN。归纳证明STRIDE_MAX - STRIDE_MIN ≤ BigStride / 2：

- 初始时所有进程stride=0，差值为0，命题成立。
- 假设某轮调度前STRIDE_MAX - STRIDE_MIN ≤ BigStride / 2成立。被选中的进程一定是stride达到STRIDE_MIN的那一个，记其新stride为STRIDE_MIN + pass。此时其它进程stride不变，所以新的最小值不大于原最大值STRIDE_MAX，新的最大值不大于max(STRIDE_MAX, STRIDE_MIN + pass)。
  - 若STRIDE_MIN + pass ≤ STRIDE_MAX，则新差值 ≤ STRIDE_MAX - 新最小值 ≤ STRIDE_MAX - STRIDE_MIN ≤ BigStride/2，命题保持。
  - 若STRIDE_MIN + pass > STRIDE_MAX，则新差值 = (STRIDE_MIN + pass) - 新最小值 ≤ pass ≤ BigStride/2，命题保持。

因此该不变量在整个运行过程中始终成立。这一性质是回绕安全比较的前提：只要差值不超过BigStride/2，就可以用有符号差值的符号判定真实大小关系。

### 3. partial_cmp实现

为Stride类型实现PartialOrd，利用上一题的不变量STRIDE_MAX - STRIDE_MIN ≤ BigStride/2，将无符号差值转为有符号数判定方向：

```rust
use core::cmp::Ordering;

struct Stride(u64);

impl PartialOrd for Stride {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // 用wrapping_sub得到模2^64意义下的差，再转有符号判方向
        let diff = self.0.wrapping_sub(other.0) as i64;
        if diff == 0 {
            Some(Ordering::Equal)
        } else if diff < 0 {
            // 真实差值落在(-BigStride/2, 0)区间，self < other
            Some(Ordering::Less)
        } else {
            // 真实差值落在(0, BigStride/2]区间，self > other
            Some(Ordering::Greater)
        }
    }
}

impl PartialEq for Stride {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
```

正确性：根据第2题，任意时刻两个就绪进程的stride差落在区间[-BigStride/2, BigStride/2]内。BigStride取2^24远小于2^63，因此self.0.wrapping_sub(other.0)恰好等于真实差值的二进制补码表示——若真实差值为正，差值的高位为0，转i64后为正；若真实差值为负，wrapping后高位被置1，转i64后为负。这样无论是否发生回绕，比较结果都与真实数轴上的先后一致。

回到第1题的8位例子：p1.stride=255、p2.stride=4。255u8.wrapping_sub(4)=251，转i8为-5，因此p1 < p2，调度器正确地选中p1。

## 荣誉准则

在完成本次实验的过程（含此前学习的过程）中，我曾分别与 **以下各位** 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

无

此外，我也参考了 **以下资料** ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

无

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按"-100"分计。

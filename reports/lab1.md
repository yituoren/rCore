# Lab1 2023010745 赵睿 计32

## 实现功能

实现了sys_trace系统调用，支持内核地址读写和系统调用次数统计。

sys_trace的syscall id为410，接收三个参数trace_request、id和data，根据trace_request的值执行不同功能：

- trace_request=0时，将id作为内存地址，读取该地址处的一个字节并返回其值。
- trace_request=1时，将id作为内存地址，将data的低8位写入该地址，返回0。
- trace_request=2时，将id作为syscall编号，返回当前任务调用该syscall的累计次数，包含本次调用。
- 其他值返回-1。

实现原理是在TaskControlBlock中增加一个大小为MAX_SYSCALL_NUM的syscall_counts数组，用于记录每个syscall的调用次数。在syscall统一分发函数的入口处，根据syscall_id对对应计数器加一，这样在实际处理sys_trace之前计数已经完成，因此trace_request=2查询自身时也能包含本次调用。内存读写则是直接将id转为裸指针进行unsafe读写操作。

## 简答作业

### 1. 在U态下执行S态特权指令或访问S态寄存器会发生什么

使用RustSBI 0.3.0-alpha.2，运行ch2b_bad_instructions和ch2b_bad_register两个测例验证：

- ch2b_bad_instructions在U态执行sret指令，内核输出 `IllegalInstruction in application, kernel killed it.`
- ch2b_bad_register在U态使用csrr读取sstatus寄存器，内核同样输出 `IllegalInstruction in application, kernel killed it.`
- 另外ch2b_bad_address向地址0x0写入数据，内核输出 `PageFault in application, bad addr = 0x0, bad instruction = 0x804003a4, kernel killed it.`

U态执行S态特权指令或访问S态CSR寄存器时，CPU触发IllegalInstruction异常，硬件将PC保存到sepc、异常原因写入scause，跳转到stvec指向的trap入口。rCore的trap_handler识别到该异常后终止应用程序。访问非法地址则触发PageFault异常，处理流程类似。

### 2. trap.S分析

#### L40 __restore的sp含义及两种使用情景

进入__restore时，sp指向内核栈上已分配的TrapContext的起始地址。

两种使用情景：

- 从trap_handler返回后直接跳转到__restore，此时sp就是__alltraps中分配TrapContext后的内核栈指针，用于恢复被中断的用户态上下文。
- 在任务切换后首次进入用户态时，内核手动构造好TrapContext并将sp设置为该TrapContext的地址，然后跳转到__restore，用于启动一个新任务。

#### L43-L48哪些寄存器被特殊处理及原因

这几行恢复了sstatus、sepc和sscratch三个CSR寄存器。

- sstatus保存了trap发生前的特权级状态，包括SPP位，决定sret后返回S态还是U态。
- sepc保存了trap发生时的用户态PC，sret指令会跳转到sepc指向的地址继续执行。
- sscratch用于保存用户栈指针，在L60的csrrw中与sp交换，恢复用户栈。

这些寄存器必须在恢复通用寄存器之前写回，因为后续恢复通用寄存器会覆盖掉临时寄存器t0/t1/t2。

#### L50-L56为什么跳过x2和x4

- x2即sp，此时sp正在被用作内核栈指针来读取TrapContext中的数据，如果提前恢复sp就无法继续从栈上加载其他寄存器了。sp在L58调整栈帧、L60 csrrw交换后才最终恢复为用户栈指针。
- x4即tp，在当前实现中应用程序不使用tp寄存器，因此没有保存也不需要恢复。

#### L60 csrrw执行后sp和sscratch的值

执行csrrw sp, sscratch, sp后：

- sp的值变为sscratch中存储的用户栈指针，即恢复到用户态栈。
- sscratch的值变为执行前sp的值，即内核栈指针，供下次trap时使用。

#### __restore中状态切换发生在哪条指令，为什么能进入用户态

状态切换发生在L61的sret指令。sret执行时，硬件会将PC设置为sepc的值，同时将特权级设置为sstatus中SPP位记录的值。因为在进入trap时SPP被硬件设为U，或者内核在构造TrapContext时将sstatus的SPP设为U，所以sret后处理器回到用户态。

#### L13 csrrw执行后sp和sscratch的值

执行csrrw sp, sscratch, sp后：

- sp的值变为sscratch中存储的内核栈指针，用于后续在内核栈上保存TrapContext。
- sscratch的值变为执行前sp的值，即用户栈指针，后续保存到TrapContext的x2槽位中。

#### 从U态进入S态是哪一条指令触发的

是用户态程序执行ecall指令触发的。ecall产生Environment Call异常，硬件自动将特权级从U提升到S，将PC保存到sepc，然后跳转到stvec指向的trap入口地址即__alltraps。

## 荣誉准则

在完成本次实验的过程（含此前学习的过程）中，我曾分别与 **以下各位** 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

无

此外，我也参考了 **以下资料** ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

无

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。

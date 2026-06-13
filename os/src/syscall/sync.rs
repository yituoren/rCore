use crate::sync::{ensure_matrix, ensure_vec, is_safe, Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;

/// helper: read current thread's tid
fn current_tid() -> usize {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid
}
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_list.len() - 1
    };
    // record this new mutex as available for the banker's algorithm
    ensure_vec(&mut process_inner.mutex_available, id + 1);
    process_inner.mutex_available[id] = 1;
    id as isize
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let tid = current_tid();
    let mid = mutex_id;
    let process = current_process();
    let mutex;
    {
        let mut inner = process.inner_exclusive_access();
        let n_res = inner.mutex_list.len();
        ensure_vec(&mut inner.mutex_available, n_res);
        ensure_matrix(&mut inner.mutex_allocation, tid, n_res);
        ensure_matrix(&mut inner.mutex_need, tid, n_res);
        if inner.deadlock_detect {
            // tentatively register the request and run the banker's check
            inner.mutex_need[tid][mid] += 1;
            let safe = is_safe(
                &inner.mutex_available,
                &inner.mutex_allocation,
                &inner.mutex_need,
            );
            if !safe {
                inner.mutex_need[tid][mid] -= 1;
                return -0xdead;
            }
        }
        mutex = Arc::clone(inner.mutex_list[mid].as_ref().unwrap());
    }
    mutex.lock();
    // request granted: move the unit from need to allocation
    let mut inner = process.inner_exclusive_access();
    if inner.deadlock_detect {
        inner.mutex_need[tid][mid] -= 1;
    }
    inner.mutex_allocation[tid][mid] += 1;
    inner.mutex_available[mid] -= 1;
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let tid = current_tid();
    let mid = mutex_id;
    let process = current_process();
    let mutex;
    {
        let mut inner = process.inner_exclusive_access();
        let n_res = inner.mutex_list.len();
        ensure_matrix(&mut inner.mutex_allocation, tid, n_res);
        if inner.mutex_allocation[tid][mid] > 0 {
            inner.mutex_allocation[tid][mid] -= 1;
        }
        ensure_vec(&mut inner.mutex_available, n_res);
        inner.mutex_available[mid] += 1;
        mutex = Arc::clone(inner.mutex_list[mid].as_ref().unwrap());
    }
    mutex.unlock();
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_list.len() - 1
    };
    // mirror semaphore count for the banker's algorithm
    ensure_vec(&mut process_inner.sem_count, id + 1);
    process_inner.sem_count[id] = res_count as i32;
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let tid = current_tid();
    let sid = sem_id;
    let process = current_process();
    let sem;
    {
        let mut inner = process.inner_exclusive_access();
        let n_res = inner.semaphore_list.len();
        ensure_matrix(&mut inner.sem_allocation, tid, n_res);
        if inner.sem_allocation[tid][sid] > 0 {
            inner.sem_allocation[tid][sid] -= 1;
        }
        ensure_vec(&mut inner.sem_count, n_res);
        inner.sem_count[sid] += 1;
        sem = Arc::clone(inner.semaphore_list[sid].as_ref().unwrap());
    }
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let tid = current_tid();
    let sid = sem_id;
    let process = current_process();
    let sem;
    {
        let mut inner = process.inner_exclusive_access();
        let n_res = inner.semaphore_list.len();
        ensure_vec(&mut inner.sem_count, n_res);
        ensure_matrix(&mut inner.sem_allocation, tid, n_res);
        ensure_matrix(&mut inner.sem_need, tid, n_res);
        if inner.deadlock_detect {
            // tentatively register the request, then check safety based on the
            // currently-free units (sem_count clamped to non-negative)
            inner.sem_need[tid][sid] += 1;
            let available: alloc::vec::Vec<i32> = inner.sem_count.iter().map(|&c| c.max(0)).collect();
            let safe = is_safe(&available, &inner.sem_allocation, &inner.sem_need);
            if !safe {
                inner.sem_need[tid][sid] -= 1;
                return -0xdead;
            }
        }
        sem = Arc::clone(inner.semaphore_list[sid].as_ref().unwrap());
    }
    sem.down();
    let mut inner = process.inner_exclusive_access();
    if inner.deadlock_detect {
        inner.sem_need[tid][sid] -= 1;
    }
    inner.sem_allocation[tid][sid] += 1;
    inner.sem_count[sid] -= 1;
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    trace!("kernel: sys_enable_deadlock_detect");
    if enabled > 1 {
        return -1;
    }
    let process = current_process();
    process.inner_exclusive_access().deadlock_detect = enabled == 1;
    0
}

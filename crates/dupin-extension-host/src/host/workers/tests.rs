use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Job, Pool, Workers, retire};

#[test]
fn a_worker_that_times_out_runs_the_job_handed_to_it_before_it_left_the_pool() {
    let (worker, jobs) = mpsc::channel::<Job>();
    let pool = Mutex::new(Pool {
        idle: vec![(7, worker)],
        next: 8,
    });
    let (ran, done) = mpsc::channel();
    let (_, handed) = pool.lock().unwrap().idle.pop().unwrap();
    handed.send(Box::new(move || ran.send(()).unwrap())).unwrap();

    let job = retire(&pool, 7, &jobs).expect("the job sent before the worker retired");
    job();
    assert_eq!(done.try_recv(), Ok(()));
}

#[test]
fn a_worker_that_times_out_with_nothing_sent_leaves_the_pool() {
    let (worker, jobs) = mpsc::channel::<Job>();
    let (other, _other_jobs) = mpsc::channel::<Job>();
    let pool = Mutex::new(Pool {
        idle: vec![(3, other), (7, worker)],
        next: 8,
    });

    assert!(retire(&pool, 7, &jobs).is_none());
    let left: Vec<_> = pool.lock().unwrap().idle.iter().map(|(id, _)| *id).collect();
    assert_eq!(left, [3]);
}

#[test]
fn every_job_runs_when_handed_out_as_workers_time_out() {
    let idle = Duration::from_micros(500);
    let workers = Workers::new(idle);
    let (ran, done) = mpsc::channel();
    let ran = Arc::new(Mutex::new(ran));
    for round in 0..400_u32 {
        let ran = ran.clone();
        workers
            .run(Box::new(move || ran.lock().unwrap().send(round).unwrap()))
            .unwrap();
        assert_eq!(
            done.recv_timeout(Duration::from_secs(5)),
            Ok(round),
            "round {round}"
        );
        std::thread::sleep(idle.mul_f64(f64::from(round % 5) / 2.0));
    }
}

#[test]
fn a_finished_worker_takes_the_next_job_instead_of_a_new_thread() {
    let workers = Workers::new(Duration::from_secs(60));
    let (ran, done) = mpsc::channel();
    let first = ran.clone();
    workers
        .run(Box::new(move || first.send(std::thread::current().id()).unwrap()))
        .unwrap();
    let first = done.recv_timeout(Duration::from_secs(5)).unwrap();
    while workers.idle() == 0 {
        std::thread::yield_now();
    }
    workers
        .run(Box::new(move || ran.send(std::thread::current().id()).unwrap()))
        .unwrap();
    assert_eq!(done.recv_timeout(Duration::from_secs(5)).unwrap(), first);
}

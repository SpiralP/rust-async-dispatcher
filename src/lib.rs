use futures::{
    channel::oneshot,
    executor::*,
    future::{BoxFuture, LocalBoxFuture},
    prelude::*,
    task::{LocalSpawnExt, SpawnExt},
};
use std::{
    fmt::Debug,
    sync::mpsc::{channel, Receiver, Sender},
};

type Task = BoxFuture<'static, ()>;

type LocalTask = LocalBoxFuture<'static, ()>;

/// Used to call futures from another thread, getting their output values via `await`.
pub struct Dispatcher {
    task_sender: Sender<Task>,
    task_receiver: Receiver<Task>,

    local_task_sender: Sender<LocalTask>,
    local_task_receiver: Receiver<LocalTask>,

    future_pool: LocalPool,
    future_spawner: LocalSpawner,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatcher {
    pub fn new() -> Self {
        let (task_sender, task_receiver) = channel();
        let (local_task_sender, local_task_receiver) = channel();

        let future_pool = LocalPool::new();
        let future_spawner = future_pool.spawner();

        Self {
            task_sender,
            task_receiver,
            local_task_sender,
            local_task_receiver,
            future_pool,
            future_spawner,
        }
    }

    fn spawn_queued(&mut self) {
        for task in self.local_task_receiver.try_iter() {
            self.future_spawner.spawn_local(task).unwrap();
        }

        for task in self.task_receiver.try_iter() {
            self.future_spawner.spawn(task).unwrap();
        }
    }

    /// Run all tasks in the pool to completion.
    ///
    /// This method will not enqueue new tasks until the next call!
    pub fn run(&mut self) {
        self.spawn_queued();

        self.future_pool.run()
    }

    /// Runs all tasks in the pool and returns if no more progress can be made on any task.
    ///
    /// This method will not enqueue new tasks until the next call!
    pub fn run_until_stalled(&mut self) {
        self.spawn_queued();

        self.future_pool.run_until_stalled()
    }

    /// Runs all tasks and returns after completing one future or until no more progress can be made. Returns true if one future was completed, false otherwise.
    ///
    /// This method will not enqueue new tasks until the next call!
    pub fn try_run_one(&mut self) -> bool {
        self.spawn_queued();

        self.future_pool.try_run_one()
    }

    /// Get a [`DispatcherHandle`] to control this Dispatcher from another thread.
    pub fn get_handle(&self) -> DispatcherHandle {
        DispatcherHandle {
            task_sender: self.task_sender.clone(),
        }
    }

    /// Get a [`LocalDispatcherHandle`] to control this Dispatcher from the same thread.
    pub fn get_handle_local(&self) -> LocalDispatcherHandle {
        LocalDispatcherHandle {
            local_task_sender: self.local_task_sender.clone(),
        }
    }
}

/// Created by calling [`Dispatcher::get_handle`].
#[derive(Clone)]
pub struct DispatcherHandle {
    task_sender: Sender<Task>,
}

impl DispatcherHandle {
    /// Enqueue a future to be ran when [`Dispatcher::run`] is called.
    ///
    /// If you want an output value from the future, use [`DispatcherHandle::dispatch`]
    pub fn spawn<F>(&mut self, future: F)
    where
        F: Future<Output = ()> + 'static + Send,
    {
        self.task_sender.send(future.boxed()).unwrap();
    }

    /// Enqueue a future to be ran when [`Dispatcher::run`] is called.
    ///
    /// This returns a Future that resolves to the input future's output value.
    pub async fn dispatch<F, O>(&mut self, future: F) -> O
    where
        F: Future<Output = O> + 'static + Send,
        O: 'static + Send + Debug,
    {
        let (return_sender, return_receiver) = oneshot::channel();

        self.spawn(
            async {
                let ret = future.await;
                return_sender.send(ret).unwrap();
            }
            .boxed(),
        );

        return_receiver.await.unwrap()
    }
}

/// Created by calling [`Dispatcher::get_handle_local`].
#[derive(Clone)]
pub struct LocalDispatcherHandle {
    local_task_sender: Sender<LocalTask>,
}

impl LocalDispatcherHandle {
    /// Enqueue a future to be ran when [`Dispatcher::run`] is called.
    ///
    /// If you want an output value from the future, use [`LocalDispatcherHandle::dispatch`]
    pub fn spawn<F>(&mut self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        self.local_task_sender.send(future.boxed_local()).unwrap();
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use futures::executor::block_on;
    use std::{thread, time::Duration};

    #[test]
    fn it_works() {
        println!(
            "dispatcher running on thread {:?}",
            thread::current().name()
        );
        let mut main_thread_dispatcher = Dispatcher::new();

        let mut handle = main_thread_dispatcher.get_handle();
        let t = thread::spawn(move || {
            block_on(async move {
                println!(
                    "new thread running on thread {:?}",
                    thread::current().name()
                );

                println!("dispatching");
                handle
                    .dispatch(async {
                        println!(
                            "dispatched task running on thread {:?}",
                            thread::current().name()
                        );

                        99
                    })
                    .await
            })
        });

        // wait for thread to dispatch
        thread::sleep(Duration::from_secs(1));

        println!("running dispatcher");
        main_thread_dispatcher.run();
        println!("dispatcher done running");

        assert!(t.join().unwrap() == 99);
    }

    #[test]
    fn it_works_local() {
        println!(
            "dispatcher running on thread {:?}",
            thread::current().name()
        );
        let mut main_thread_dispatcher = Dispatcher::new();

        let mut local_handle = main_thread_dispatcher.get_handle_local();

        let mut inner_local_handle = local_handle.clone();
        println!("spawning");
        local_handle.spawn(async move {
            println!(
                "spawned task running on same thread {:?}",
                thread::current().name()
            );

            inner_local_handle.spawn(async {
                println!("inner running");
            });
        });

        println!("running dispatcher");
        main_thread_dispatcher.run();
        println!("dispatcher done running");

        println!("running dispatcher again");
        main_thread_dispatcher.run();
        println!("dispatcher done running again");
    }

    #[test]
    fn test_tokio() {
        // you can't use tokio timers in these threads though!

        let mut main_thread_dispatcher = Dispatcher::new();
        let mut handle = main_thread_dispatcher.get_handle();

        let tokio_thread = thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();

            rt.block_on(async move {
                println!("dispatching");
                handle
                    .dispatch(async {
                        println!(
                            "dispatched task running on thread {:?}",
                            thread::current().name()
                        );

                        99u8
                    })
                    .await
            })
        });

        // wait for thread to dispatch
        thread::sleep(Duration::from_secs(1));

        println!("running dispatcher");
        main_thread_dispatcher.run();
        println!("dispatcher done running");

        assert!(tokio_thread.join().unwrap() == 99);
    }
}

use crossbeam_channel::{unbounded, Receiver, Sender};
use futures::channel::oneshot::channel as oneshot_channel;

type Task = Box<dyn FnOnce() + Send>;

/// Used to call functions from another thread, getting their return values via `await`.
pub struct Dispatcher {
  task_sender: Sender<Task>,
  task_receiver: Receiver<Task>,
}

impl Default for Dispatcher {
  fn default() -> Self {
    Self::new()
  }
}

impl Dispatcher {
  pub fn new() -> Self {
    let (task_sender, task_receiver) = unbounded();

    Self {
      task_sender,
      task_receiver,
    }
  }

  /// Run all functions currently in the queue.
  pub fn process_tasks(&mut self) {
    for task in self.task_receiver.try_iter() {
      task();
    }
  }

  /// Get a [`DispatcherHandle`] to control this Dispatcher from another thread.
  pub fn get_handle(&self) -> DispatcherHandle {
    DispatcherHandle {
      task_sender: self.task_sender.clone(),
    }
  }
}

/// Created by calling [`Dispatcher::get_handle`].
pub struct DispatcherHandle {
  task_sender: Sender<Task>,
}

impl DispatcherHandle {
  fn spawn<F: FnOnce() + Send + 'static>(&mut self, f: F) {
    self.task_sender.send(Box::new(f)).unwrap();
  }

  /// Enqueue a function to be ran when [`Dispatcher::process_tasks`] is called.
  ///
  /// This returns a Future that resolves to the function's return value.
  pub async fn dispatch<T, F>(&mut self, f: F) -> T
  where
    T: Send + 'static,
    F: (FnOnce() -> T) + Send + 'static,
  {
    let (oneshot_sender, oneshot_receiver) = oneshot_channel();

    self.spawn(move || {
      let _ignore_error = oneshot_sender.send(f());
    });

    oneshot_receiver.await.unwrap()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use futures::executor::block_on;
  use std::{thread, time::Duration};

  #[test]
  fn it_works() {
    let mut main_thread_dispatcher = Dispatcher::new();
    let mut handle = main_thread_dispatcher.get_handle();

    let t = thread::spawn(move || {
      block_on(async move {
        handle
          .dispatch(move || {
            println!("MAIN THREAD HI");

            99
          })
          .await
      })
    });

    thread::sleep(Duration::from_millis(1000));
    main_thread_dispatcher.process_tasks();

    assert!(t.join().unwrap() == 99);
  }
}

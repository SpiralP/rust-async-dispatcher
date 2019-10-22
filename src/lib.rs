use crossbeam_channel::{unbounded, Receiver, Sender};
use futures::channel::oneshot::channel as oneshot_channel;

type Task = Box<dyn FnOnce() + Send>;

pub struct Dispatcher {
  sender: Sender<Task>,
  receiver: Receiver<Task>,
}

impl Default for Dispatcher {
  fn default() -> Self {
    Self::new()
  }
}

impl Dispatcher {
  pub fn new() -> Self {
    let (sender, receiver) = unbounded();

    Self { sender, receiver }
  }

  pub fn process_tasks(&mut self) {
    for task in self.receiver.try_iter() {
      task();
    }
  }

  pub fn get_handle(&self) -> DispatcherHandle {
    DispatcherHandle {
      sender: self.sender.clone(),
    }
  }
}

pub struct DispatcherHandle {
  sender: Sender<Task>,
}

impl DispatcherHandle {
  pub fn spawn<F: FnOnce() + Send + 'static>(&mut self, f: F) {
    self.sender.send(Box::new(f)).unwrap();
  }

  pub async fn spawn_wait<T: Send + 'static, F: (FnOnce() -> T) + Send + 'static>(
    &mut self,
    f: F,
  ) -> T {
    let (oneshot_sender, oneshot_receiver) = oneshot_channel();

    self.spawn(move || {
      let ret = f();

      let _ignore_error = oneshot_sender.send(ret);
    });

    oneshot_receiver.await.unwrap()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn it_works() {
    let mut main_thread_dispatcher = Dispatcher::new();
    let mut handle = main_thread_dispatcher.get_handle();

    std::thread::spawn(move || {
      futures::executor::block_on(async move {
        let result = handle
          .spawn_wait(move || {
            println!("MAIN THREAD HI");

            99
          })
          .await;

        println!("thread got {}", result);
      });
    });

    for _ in 0..4 {
      main_thread_dispatcher.process_tasks();
      std::thread::sleep_ms(100);
    }
  }
}

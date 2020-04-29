extern crate cgc;
use cgc::heap::*;
use cgc::threads::*;

fn main() {
    simple_logger::init().unwrap();
    attach_current_thread();
    {
        let x = mt_alloc(42, false);
        let y = mt_alloc(3, false);
        println!("{}", *x + *y);
    }
    HEAP.collect();
    std::thread::sleep(std::time::Duration::from_nanos(200000));
}

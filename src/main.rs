extern crate cgc;

use cgc::collector::*;
use cgc::rooting::*;
use cgc::trace::*;

struct Foo {
    x: i32,
    next: Option<Heap<Foo>>,
}
impl Traceable for Foo {
    fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        println!("trace foo {}", self.x);
        match &mut self.next {
            Some(x) => f(x),
            _ => (),
        }
    }
}

impl Finalizer for Foo {
    fn finalize(&mut self) {
        println!("finalize foo {}", self.x);
    }
}

fn main() {
    let n = time::Instant::now();
    let mut gc = GlobalCollector::new(1024 * 1024 * 100);
    for _ in 0..1000000 {
        gc.alloc(42usize);
    }

    gc.compact();

    println!("{}", n.elapsed().whole_milliseconds());
}

/*
fn main() {
    let mut data = Vec::with_capacity(1000000);
    let n = time::Instant::now();
    for _ in 0..1000000 {
        data.push(unsafe { std::alloc::alloc(std::alloc::Layout::new::<usize>()) });
    }

    for x in data.iter() {
        unsafe {
            std::alloc::dealloc(*x, std::alloc::Layout::new::<usize>());
        }
    }

    println!("{}", n.elapsed().whole_milliseconds());
}
*/

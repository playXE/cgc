# cgc 
cgc is a copying garbage collector.

# Advantages
- Fast.
- Usable in a real-time thread, because collection can occur in another thread. 
- Works fine with Rust memory management.
- Thread safe (not yet)

# Disadvantages
- Not yet thread safe,you must be sure that you do thread-safe things with `GCValue`.
  If you will allocate not thread-safe type and run GC in other thread it may break your code.

- You need define `Collectable` trait for every type that you want to GC.

# Why copying GC?
 This kind of GC is very simple to implement and use and got good perfomance,and there are only one problem: memory usage may be twice as high compared to other algorithms.

# How to use

Just add `cgc` to your crate dependencies:
```toml
[dependencies]
cgc = "*"
```

And then start coding:
```rust
use cgc::{gc_allocate,Collectable,GCValue};

pub struct A;

impl Collectable for A {
    fn size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

pub struct Foo(Vec<i32>);

impl Collectable {
    fn child(&self) -> Vec<GCValue<dyn Collectable>> {
        self.0.iter().map(|x| *x).collect() 
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

fn main() {
    let foo = gc_allocate(Foo(vec![]));

    foo.borrow_mut().0.push(A);
}

```
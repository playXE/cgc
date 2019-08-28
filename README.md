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

- You need define `Trace` trait for every type that you want to GC.

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

use cgc::*;


pub struct Foo(GC<i32>);

impl Trace for Foo {
    fn trace(&self) {
        self.0.mark();
    }
}

```


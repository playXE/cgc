# Barriers in cgc
Since cgc is concurrent and moving garbage collector it needs read and write barriers to work. This section covers how to use GC barriers and how they work.

## Read barriers
Read barriers is implicit and programmer should not care about them. Read barrier is emitted when you try to load value and what read barrier does is returns proper pointer to heap value.

Implementation of read barriers is pretty simple: 
```rust
fn read_barrier(src) {
    return forward(src)
}
```
We assume that forwarding pointer of `src` points to `src` itself or into new address if GC is copied object and what read barrier does is reads forward pointer.

## Write barriers
Write barriers is explicit and programmer should care about them otherwise this may lead to UB or segfault.
Write barriers should be inserted before any store operation into heap value: 
```rust
let value = mt_alloc(vec![None],true);
cgc::write_barrier(&value);
value.get_mut()[0] = Some(42);
```
Write barrier helps GC to rescan object if other GC object is stored into other GC object.

Implementation:
```rust
fn write_barrier(src) {
    if copying_in_progress {
        if !is_grey(src) {
            worklist.push(src);
        } else {
            return;
        }
    }
}
```

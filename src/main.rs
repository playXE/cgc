use cgc::generational::*;

fn main() {
    let v = gc_allocate(vec![]);
    gc_add_root(v);
    v.borrow_mut().push(vec![gc_allocate(String::from("Hello,World!"))]);
    gc_enable_stats();
    println!("HELLO");
    gc_collect_not_par();
}

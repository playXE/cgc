# Allocation
Concurrent version of cgc does not allow creating your own instance of heap, instead there are global heap with 32kb per heap block and support for larget allocationg (allocation bigger than 8kb). There are `GlobalHeap::allocate` function but you should not use it, instead you need to use `mt_alloc` function from `cgc::threads`. 

## mt_alloc(value,finalize)
`mt_alloc` accepts two arguments: first one is value you want to place on heap and second one is boolean flag whether should GC invoke value finalizer and destructor or no. This boolean flag should be true for *all* structures that allocate on heap,otherwise memory leaks will happen.

## mt_root(handle)
`mt_root` takes `Handle<T>` and makes rooted value from it, this function is usefull if you want to put your value into rootset.
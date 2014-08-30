extern crate rlibc;

use std::iter::{Chain, range_inclusive};
use std::mem;
use std::raw;
use std::slice;

fn vec_copy<T>(v: &mut Vec<T>, to: uint, from: uint, n: uint) {
  unsafe {
    let p_to = v.as_mut_ptr().offset(to as int);
    let p_from = v.as_ptr().offset(from as int);
    rlibc::memcpy(p_to as *mut u8, p_from as *const u8, n * mem::size_of::<T>());
  }
}

/// Circular bounded queue.
pub struct Queue<T> {
  pub contents: Vec<T>,
  /// index in `contents` the queue starts at
  pub head: uint,
  /// index in `contents` to place the next element at
  pub tail: uint,
  /// number of elements used
  pub length: uint,
}

impl<T: Clone> Queue<T> {
  pub fn new(capacity: uint) -> Queue<T> {
    Queue {
      contents: Vec::with_capacity(capacity),
      head: 0,
      tail: 0,
      length: 0,
    }
  }

  pub fn push(&mut self, t: T) {
    assert!(
      self.length < self.contents.capacity(),
      "Bounded queue (capacity {}) exceeded",
      self.contents.capacity()
    );

    if self.contents.len() < self.contents.capacity() {
      self.contents.push(t);
    } else {
      *self.contents.get_mut(self.tail) = t;
    }

    self.tail = (self.tail + 1) % self.contents.capacity();
    self.length += 1;
  }

  pub fn push_all<'a, I: Iterator<T>>(&mut self, ts: I) {
    let mut ts = ts;
    for t in ts {
      self.push(t);
    }
  }

  pub fn pop(&mut self, count: uint) {
    assert!(count <= self.length);
    self.head = (self.head + count) % self.contents.capacity();
    self.length -= count;
  }

  pub fn is_empty(&self) -> bool {
    self.length == 0
  }

  /// Swap `count` elements from `idx` with the last `count` elements of the
  /// queue, then drop the last `count` elements.
  /// The ordering amongst those `count` elements is maintained.
  pub fn swap_remove(&mut self, idx: uint, count: uint) {
    assert!(count <= self.length);
    self.length -= count;

    assert!(idx <= self.length);

    self.tail = (self.tail + self.contents.capacity() - count) % self.contents.capacity();

    if idx < self.length {
      assert!(
        idx + count <= self.length,
        "Queue::swap_remove in overlapping regions"
      );

      // At this point, we have a guarantee that the regions do not overlap.
      // Therefore, either the copy-from or copy-to regions might be broken up
      // by the end of the vector, but not both.

      let buffer_wrap_point = self.contents.capacity() - count;
      if idx > buffer_wrap_point {
        // copy TO an area of the queue that crosses the end of the buffer.

        let count1 = self.contents.capacity() - idx;
        vec_copy(&mut self.contents, idx, self.tail, count1);
        let count2 = count - count1;
        vec_copy(&mut self.contents, 0, self.tail + count1, count2);
      } else if self.tail > buffer_wrap_point {
        // copy FROM an area of the queue that crosses the end of the buffer.

        let count1 = self.contents.capacity() - self.tail;
        vec_copy(&mut self.contents, idx, self.tail, count1);
        let count2 = count - count1;
        vec_copy(&mut self.contents, idx + count1, 0, count2);
      } else {
        vec_copy(&mut self.contents, idx, self.tail, count);
      }
    }
  }

  pub fn len(&self) -> uint {
    self.length
  }

  #[allow(dead_code)]
  pub fn clear(&mut self) {
    self.contents.clear();
    self.head = 0;
    self.tail = 0;
    self.length = 0;
  }

  pub fn iter<'a>(&'a self, low: uint, high: uint) -> QueueItems<'a, T> {
    let (l, h) = self.slices(low, high);
    assert!(l.len() + h.len() <= self.len());
    QueueItems { inner: l.iter().chain(h.iter()) }
  }

  pub fn slices<'a>(&'a self, low: uint, high: uint) -> (&'a [T], &'a [T]) {
    assert!(low <= self.length);
    assert!(high <= self.length);
    let head = (self.head + low) % self.contents.capacity();
    let tail = (self.head + high) % self.contents.capacity();
    if head <= tail {
      (self.contents.slice(head, tail), self.contents.slice(tail, tail))
    } else {
      (self.contents.slice(head, self.contents.capacity()), self.contents.slice(0, tail))
    }
  }
}

#[test]
fn push_then_slice() {
  let elems: Vec<int> = Vec::from_slice([1, 2, 3]);
  let mut q = Queue::new(32);
  q.push_all(elems.as_slice());
  assert!(q.len() == elems.len());
  let (l, h) = q.slices(0, q.len() - 1);
  assert!(l.len() + h.len() == elems.len() - 1);
  for (i, elem) in l.iter().enumerate() {
    assert!(*elem == elems[i]);
  }
  for (i, elem) in h.iter().enumerate() {
    assert!(*elem == elems[i + l.len()]);
  }
}

#[test]
fn push_then_pop() {
  let popped_pushes: Vec<int> = Vec::from_slice([1, 2, 3]);
  let more_pushes: Vec<int> = Vec::from_slice([4, 5]);
  let mut q = Queue::new(32);
  q.push_all(popped_pushes.as_slice());
  q.push_all(more_pushes.as_slice());
  assert!(q.len() == popped_pushes.len() + more_pushes.len());
  q.pop(popped_pushes.len());
  assert!(q.len() == more_pushes.len());
  let (l, h) = q.slices(0, q.len());
  assert!(l.len() + h.len() == more_pushes.len());
  for (i, elem) in l.iter().enumerate() {
    assert!(*elem == more_pushes[i]);
  }
  for (i, elem) in h.iter().enumerate() {
    assert!(*elem == more_pushes[i + l.len()]);
  }
}

#[test]
fn wrapped_pushes() {
  static capacity: uint = 32;
  let popped_pushes = Vec::from_elem(capacity / 2, 0);
  let more_pushes: Vec<int> = range_inclusive(1, capacity as int).collect();
  let mut q: Queue<int> = Queue::new(capacity);
  q.push_all(popped_pushes.as_slice());
  q.pop(popped_pushes.len());
  q.push_all(more_pushes.as_slice());
  assert!(q.len() == more_pushes.len());
  let (l, h) = q.slices(0, q.len());
  assert!(l.len() + h.len() == more_pushes.len());
  assert!(h.len() > 0);
  for (i, elem) in l.iter().enumerate() {
    assert!(*elem == more_pushes[i]);
  }
  for (i, elem) in h.iter().enumerate() {
    assert!(*elem == more_pushes[i + l.len()]);
  }
}

pub struct QueueItems<'a, T> { inner: Chain<slice::Items<'a, T>, slice::Items<'a, T>> }

impl<'a, T> Iterator<&'a T> for QueueItems<'a, T> {
  fn next(&mut self) -> Option<&'a T> { self.inner.next() }
  fn size_hint(&self) -> (uint, Option<uint>) { self.inner.size_hint() }
}
//! A stack that holds its first few items in place and only past them allocates, for the walks over a call's inputs
//! and answers: most calls reach no deeper than a few structs, and those allocate nothing for their walk.

/// How many items a walk holds before it allocates.
const IN_PLACE: usize = 8;

/// A stack holding its first [`IN_PLACE`] items in place and the rest on the heap, each item at the index it was
/// pushed at until it is popped.
pub(crate) struct Stack<T> {
    in_place: [Option<T>; IN_PLACE],
    beyond: Vec<T>,
    length: usize,
}

impl<T> Stack<T> {
    pub(crate) fn new() -> Stack<T> {
        Stack {
            in_place: [const { None }; IN_PLACE],
            beyond: Vec::new(),
            length: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.length
    }

    pub(crate) fn push(&mut self, item: T) {
        if self.length < IN_PLACE {
            self.in_place[self.length] = Some(item);
        } else {
            self.beyond.push(item);
        }
        self.length += 1;
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.length == 0 {
            return None;
        }
        self.length -= 1;
        if self.length < IN_PLACE {
            self.in_place[self.length].take()
        } else {
            self.beyond.pop()
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        if index >= self.length {
            return None;
        }
        if index < IN_PLACE {
            self.in_place[index].as_ref()
        } else {
            self.beyond.get(index - IN_PLACE)
        }
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.length {
            return None;
        }
        if index < IN_PLACE {
            self.in_place[index].as_mut()
        } else {
            self.beyond.get_mut(index - IN_PLACE)
        }
    }
}

impl<T> std::ops::Index<usize> for Stack<T> {
    type Output = T;

    fn index(&self, index: usize) -> &T {
        self.get(index)
            .expect("a walk reads only an item it pushed and has not popped")
    }
}

impl<T> std::ops::IndexMut<usize> for Stack<T> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        self.get_mut(index)
            .expect("a walk changes only an item it pushed and has not popped")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_past_those_held_in_place_keep_their_order_and_index() {
        let mut stack = Stack::new();
        for item in 0..20 {
            stack.push(item);
        }
        assert_eq!(stack.len(), 20);
        assert_eq!(stack.get(3), Some(&3));
        assert_eq!(stack.get(15), Some(&15));
        assert_eq!(stack.get(20), None);
        *stack.get_mut(12).unwrap() = 120;
        let popped: Vec<i32> = std::iter::from_fn(|| stack.pop()).collect();
        assert_eq!(popped[..9], [19, 18, 17, 16, 15, 14, 13, 120, 11]);
        assert_eq!(popped.len(), 20);
        assert_eq!(stack.pop(), None);
    }
}

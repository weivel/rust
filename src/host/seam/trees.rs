//! A value tree as bytes, the form a method's arguments and its answer cross the seam in: one tag byte per node, a
//! number as its eight little-endian bytes, and a text, a list or a record led by its length as eight little-endian
//! bytes. No JSON crosses the seam, as no JSON crosses a C boundary of weivel's.

use super::abi::Answering;
use crate::core::Answer;
use crate::host::description::Tree;
use crate::host::methods::MethodResult;

const ANSWERED_NOTHING: u64 = 0;
const ANSWERED_A_TREE: u64 = 1;
const FAILED: u64 = 2;

const NULL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const NUMBER: u8 = 3;
const TEXT: u8 = 4;
const LIST: u8 = 5;
const RECORD: u8 = 6;

/// The deepest a tree crossing the seam may nest, the root its first level: the standard's JSON reader's own limit, so
/// every tree a document holds crosses, and so a hostile or broken one never overflows a thread's stack, its encoding,
/// its decoding and its drop each recursing once per level.
pub(super) const DEPTH_LIMIT: usize = 256;

fn too_deep() -> String {
    format!("a value tree crossing the seam nests deeper than {DEPTH_LIMIT} levels")
}

/// `tree` in the seam's encoding, refusing one nested deeper than the limit.
pub(super) fn encode(tree: &Tree) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    encode_into(tree, &mut bytes, 1)?;
    Ok(bytes)
}

fn encode_length(length: usize, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&(length as u64).to_le_bytes());
}

fn encode_text(text: &str, bytes: &mut Vec<u8>) {
    encode_length(text.len(), bytes);
    bytes.extend_from_slice(text.as_bytes());
}

fn encode_into(tree: &Tree, bytes: &mut Vec<u8>, depth: usize) -> Result<(), String> {
    if depth > DEPTH_LIMIT {
        return Err(too_deep());
    }
    match tree {
        Tree::Null => bytes.push(NULL),
        Tree::Boolean(false) => bytes.push(FALSE),
        Tree::Boolean(true) => bytes.push(TRUE),
        Tree::Number(number) => {
            bytes.push(NUMBER);
            bytes.extend_from_slice(&number.to_le_bytes());
        }
        Tree::Text(text) => {
            bytes.push(TEXT);
            encode_text(text, bytes);
        }
        Tree::List(items) => {
            bytes.push(LIST);
            encode_length(items.len(), bytes);
            for item in items {
                encode_into(item, bytes, depth + 1)?;
            }
        }
        Tree::Record(entries) => {
            bytes.push(RECORD);
            encode_length(entries.len(), bytes);
            for (key, value) in entries {
                encode_text(key, bytes);
                encode_into(value, bytes, depth + 1)?;
            }
        }
    }
    Ok(())
}

/// Reads one tree in the seam's encoding, refusing bytes that do not hold exactly one.
pub(super) fn decode(bytes: &[u8]) -> Result<Tree, String> {
    let mut reading = Reading { bytes, at: 0 };
    let tree = reading.tree(1)?;
    if reading.at != bytes.len() {
        return Err("a value tree crossed the seam with bytes after it".to_owned());
    }
    Ok(tree)
}

struct Reading<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reading<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], String> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| "a value tree crossed the seam cut short".to_owned())?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn eight(&mut self) -> Result<[u8; 8], String> {
        let mut word = [0; 8];
        word.copy_from_slice(self.take(8)?);
        Ok(word)
    }

    fn length(&mut self) -> Result<usize, String> {
        let length = u64::from_le_bytes(self.eight()?);
        let left = (self.bytes.len() - self.at) as u64;
        if length > left {
            return Err("a value tree crossed the seam with a length past its end".to_owned());
        }
        Ok(length as usize)
    }

    fn text(&mut self) -> Result<String, String> {
        let length = self.length()?;
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| "a value tree crossed the seam with a text that is not UTF-8".to_owned())
    }

    fn tree(&mut self, depth: usize) -> Result<Tree, String> {
        if depth > DEPTH_LIMIT {
            return Err(too_deep());
        }
        let tag = self.take(1)?[0];
        match tag {
            NULL => Ok(Tree::Null),
            FALSE => Ok(Tree::Boolean(false)),
            TRUE => Ok(Tree::Boolean(true)),
            NUMBER => Ok(Tree::Number(f64::from_le_bytes(self.eight()?))),
            TEXT => Ok(Tree::Text(self.text()?)),
            LIST => {
                let count = self.length()?;
                (0..count)
                    .map(|_| self.tree(depth + 1))
                    .collect::<Result<_, _>>()
                    .map(Tree::List)
            }
            RECORD => {
                let count = self.length()?;
                (0..count)
                    .map(|_| Ok((self.text()?, self.tree(depth + 1)?)))
                    .collect::<Result<_, String>>()
                    .map(Tree::Record)
            }
            other => Err(format!(
                "a value tree crossed the seam with the unknown tag {other}"
            )),
        }
    }
}

/// How a method the other side called ended, as the seam answers it; a name it does not offer answers absent.
pub(super) fn answering_of(result: Option<MethodResult>) -> Result<Answering, Answer> {
    match result.ok_or(Answer::Absent)? {
        Ok(None) => Ok(Answering {
            number: ANSWERED_NOTHING,
            ..Answering::default()
        }),
        Ok(Some(tree)) => match encode(&tree) {
            Ok(bytes) => Ok(Answering {
                number: ANSWERED_A_TREE,
                bytes,
                ..Answering::default()
            }),
            Err(failure) => Ok(Answering {
                number: FAILED,
                texts: vec![format!("the method answered what cannot cross: {failure}")],
                ..Answering::default()
            }),
        },
        Err(failure) => Ok(Answering {
            number: FAILED,
            texts: vec![failure],
            ..Answering::default()
        }),
    }
}

/// How the method `name` called across the seam ended, read from what the other side answered.
pub(super) fn result_of(answered: Result<Answering, Answer>, name: &str) -> MethodResult {
    let answered = match answered {
        Ok(answered) => answered,
        Err(Answer::Absent) => return Err(format!("the plugin offers no method {name}")),
        Err(failure) => return Err(format!("the method {name} could not be called: {failure}")),
    };
    match answered.number {
        ANSWERED_NOTHING => Ok(None),
        ANSWERED_A_TREE => decode(&answered.bytes).map(Some),
        _ => Err(answered.texts.into_iter().next().unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_reads_back_as_it_was_written_with_its_record_order_kept() {
        let tree = Tree::Record(vec![
            ("user".to_owned(), Tree::Text("someone".to_owned())),
            ("remember".to_owned(), Tree::Boolean(true)),
            (
                "list".to_owned(),
                Tree::List(vec![Tree::Null, Tree::Number(-2.5), Tree::Boolean(false)]),
            ),
        ]);
        assert_eq!(decode(&encode(&tree).unwrap()), Ok(tree));
    }

    #[test]
    fn bytes_cut_short_or_past_one_tree_are_refused() {
        let bytes = encode(&Tree::Text("long enough".to_owned())).unwrap();
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes.clone();
        longer.push(NULL);
        assert!(decode(&longer).is_err());
    }

    fn nested(levels: usize) -> Tree {
        (1..levels).fold(Tree::Null, |inner, _| Tree::List(vec![inner]))
    }

    /// The bytes of `levels` nested lists, written by hand, so a depth past the limit can be handed to the decoder.
    fn nested_bytes(levels: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        for _ in 1..levels {
            bytes.push(LIST);
            bytes.extend_from_slice(&1_u64.to_le_bytes());
        }
        bytes.push(NULL);
        bytes
    }

    #[test]
    fn a_tree_at_the_limit_crosses_on_a_two_megabyte_thread_and_one_past_it_is_refused() {
        let crossed = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let tree = nested(DEPTH_LIMIT);
                let bytes = encode(&tree).expect("a tree at the limit encodes");
                let read = decode(&bytes).expect("a tree at the limit decodes");
                let same = read == tree;
                drop((tree, read));
                let refused_encode = encode(&nested(DEPTH_LIMIT + 1)).is_err();
                let refused_decode = decode(&nested_bytes(DEPTH_LIMIT + 1)).is_err();
                let deep_refused = decode(&nested_bytes(100_000)).is_err();
                (same, refused_encode, refused_decode, deep_refused)
            })
            .expect("the thread starts")
            .join()
            .expect("nothing overflows the thread's stack");
        assert_eq!(crossed, (true, true, true, true));
    }
}

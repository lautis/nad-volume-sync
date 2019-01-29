#[derive(Debug, PartialEq, Eq)]
pub enum OpCode {
    Poll,
    Source,
    Volume,
    Power,
    Mute,
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReceiverFrame {
    pub command: OpCode,
    pub payload: u8,
}

impl From<u8> for OpCode {
    fn from(opcode: u8) -> OpCode {
        match opcode {
            2 => OpCode::Poll,
            3 => OpCode::Source,
            4 => OpCode::Volume,
            9 => OpCode::Power,
            10 => OpCode::Mute,
            _ => OpCode::Unknown,
        }
    }
}

named!(pub parse_frame<&[u8], ReceiverFrame>, do_parse!(
    bits!(tag_bits!(u8, 8, 0x0)) >>
    bits!(tag_bits!(u8, 8, 0x1)) >>
    bits!(tag_bits!(u8, 8, 0x2)) >>
    command: take!(1) >>
    payload: take!(1) >>
    (ReceiverFrame {
        command: OpCode::from(command[0]),
        payload: payload[0]
    })
));

named!(pub parse_frames<&[u8], Vec<ReceiverFrame>>,
    many0!(complete!(parse_frame)));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_volume() {
        assert_eq!(
            ReceiverFrame {
                command: OpCode::Volume,
                payload: 0xf0
            },
            parse_frame(&vec![0x0, 0x1, 0x2, 0x4, 0xf0]).unwrap().1
        );
    }

    #[test]
    fn parse_multiple() {
        assert_eq!(
            vec![
                ReceiverFrame {
                    command: OpCode::Volume,
                    payload: 0xf0
                },
                ReceiverFrame {
                    command: OpCode::Power,
                    payload: 0x01
                },
            ],
            parse_frames(&vec![0x0, 0x1, 0x2, 0x4, 0xf0, 0x0, 0x1, 0x2, 0x9, 0x01])
                .unwrap()
                .1
        );
    }
}

use byteorder::{BigEndian, ReadBytesExt};
use std::{
    collections::{BTreeMap, HashMap},
    fmt::{self, Display},
    io::{Cursor, Read, Seek, SeekFrom},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    num::TryFromIntError,
};

pub struct Mmdb<T: Read + Seek> {
    reader: T,
    metadata: MmdbMetadata,
}

#[derive(Debug, thiserror::Error)]
pub enum MmdbError {
    #[error("Database does not contain metadata marker")]
    MetadataNotFound,
    #[error("Metadata does not contain needed field")]
    InvalidMetadata(&'static str),
    #[error("Data is malformed")]
    InvalidData(&'static str),
    #[error("Wrong database type (eg. attempting to query IPv6 address on IPv4 database)")]
    WrongDatabaseType,
    #[error("IO error encountered while reading database")]
    BadIo(#[from] std::io::Error),
    #[error("Int conversion error encountered while reading database")]
    BadConversion(#[from] TryFromIntError),
    #[error("Feature of MMDB is not implemented for this reader")]
    NotImplemented(&'static str),
}

impl<T: Read + Seek> Mmdb<T> {
    pub fn new(mut reader: T) -> Result<Mmdb<T>, MmdbError> {
        reader.seek(std::io::SeekFrom::End(0))?;
        let file_size = reader.stream_position()?;

        let start_byte = match file_size {
            0..128_000 => 0,
            128_000.. => file_size - 128_000,
        };

        let tail_len = (file_size - start_byte) as usize;
        let mut contents = vec![0u8; tail_len];
        reader.seek(SeekFrom::Start(start_byte))?;
        reader.read_exact(&mut contents)?;

        static METADATA_MARKER: &[u8] = b"\xAB\xCD\xEFMaxMind.com";

        let Some(marker_pos) = contents
            .windows(METADATA_MARKER.len())
            .rposition(|x| x == METADATA_MARKER)
        else {
            return Err(MmdbError::MetadataNotFound);
        };
        let marker_pos = marker_pos + METADATA_MARKER.len();
        let mut contents = Cursor::new(contents);
        contents.seek(SeekFrom::Start(marker_pos as u64))?;

        let typ = read_type(&mut contents, None)?;
        let metadata = MmdbMetadata::new(&typ)?;
        reader.seek(SeekFrom::Start(0))?;
        Ok(Self { reader, metadata })
    }

    pub fn query_ip(&mut self, ip: impl Into<IpAddr>) -> Result<Option<Type>, MmdbError> {
        let ip = ip.into();
        match ip {
            IpAddr::V4(ip) => self.query_ipv4(ip),
            IpAddr::V6(ip) => self.query_ipv6(ip),
        }
    }

    pub fn query_ipv4(&mut self, ip: impl Into<Ipv4Addr>) -> Result<Option<Type>, MmdbError> {
        match self.metadata.ip_version {
            4 => {
                let ip = ip.into();
                let ip = ip.to_bits();
                self.query_ip_uint(ip as u128, 32)
            }
            6 => {
                let ip = ip.into();
                let ip = ip.to_ipv6_compatible();
                self.query_ipv6(ip)
            }
            _ => Err(MmdbError::InvalidMetadata(
                "database has invalid ip version",
            )),
        }
    }

    pub fn query_ipv6(&mut self, ip: impl Into<Ipv6Addr>) -> Result<Option<Type>, MmdbError> {
        match self.metadata.ip_version {
            4 => Err(MmdbError::WrongDatabaseType),
            6 => {
                let ip = ip.into();
                let ip = ip.to_bits();
                self.query_ip_uint(ip, 128)
            }
            _ => Err(MmdbError::InvalidMetadata(
                "database has invalid ip version",
            )),
        }
    }

    pub fn query_ip_uint(&mut self, ip: u128, num_bits: usize) -> Result<Option<Type>, MmdbError> {
        self.reader.seek(SeekFrom::Start(0))?;
        for i in (0..num_bits).rev() {
            let bit = match (ip >> i) & 1 {
                1 => true,
                0 => false,
                _ => unreachable!(),
            };

            match read_record(&mut self.reader, &self.metadata, bit)? {
                RecordReadResult::TraverseTreeTo(pos) => {
                    self.reader.seek(SeekFrom::Start(pos as u64))?;
                }
                RecordReadResult::Data(pos) => {
                    self.reader.seek(SeekFrom::Start(pos as u64))?;
                    let typ = read_type(&mut self.reader, Some(&self.metadata))?;
                    return Ok(Some(typ));
                }
                RecordReadResult::NoData => return Ok(None),
            }
        }
        Ok(None)
    }
}

#[derive(Clone, Debug)]
enum RecordReadResult {
    TraverseTreeTo(usize),
    NoData,
    Data(usize),
}

fn read_record<T: Read + Seek>(
    reader: &mut T,
    metadata: &MmdbMetadata,
    bit_set: bool,
) -> Result<RecordReadResult, MmdbError> {
    let bytes_per_node = bytes_per_node(metadata.record_size)?;
    // let left_record = reader.read_uint128::<BigEndian>(bytes as usize)?;
    // let right_record = reader.read_uint128::<BigEndian>(bytes as usize)?;

    let (left_record, right_record) = match bytes_per_node {
        6 => {
            let left = reader.read_u24::<BigEndian>()?;
            let right = reader.read_u24::<BigEndian>()?;
            (left, right)
        }
        7 => {
            let mut buf = [0u8; 7];
            reader.read_exact(&mut buf)?;

            let left = (((buf[3] as u32) & 0xf0) << 20)
                | ((buf[0] as u32) << 16)
                | ((buf[1] as u32) << 8)
                | (buf[2] as u32);

            let right = (((buf[3] as u32) & 0x0f) << 24)
                | ((buf[4] as u32) << 16)
                | ((buf[5] as u32) << 8)
                | (buf[6] as u32);

            (left, right)
        }
        8 => {
            let left = reader.read_u32::<BigEndian>()?;
            let right = reader.read_u32::<BigEndian>()?;
            (left, right)
        }
        _ => {
            return Err(MmdbError::InvalidData("bad node size"));
        }
    };

    // println!(
    //     "{left_record} {right_record} {bit_set} {}",
    //     metadata.node_count
    // );

    let selected_record = match bit_set {
        false => left_record as u128,
        true => right_record as u128,
    };
    let node_count = metadata.node_count as u128;

    if selected_record < node_count {
        Ok(RecordReadResult::TraverseTreeTo(
            selected_record as usize * bytes_per_node as usize,
        ))
    } else if selected_record == node_count {
        Ok(RecordReadResult::NoData)
    } else {
        let data_section_offset = selected_record - node_count - 16;
        let search_tree_size = bytes_per_node as u128 * node_count;
        let file_offset = data_section_offset + search_tree_size + 16;
        Ok(RecordReadResult::Data(file_offset as usize))
    }
}

#[derive(Debug, Clone)]
struct MmdbMetadata {
    node_count: u32,
    record_size: u16,
    ip_version: u16,
}

impl MmdbMetadata {
    fn new(metadata_map: &Type) -> Result<Self, MmdbError> {
        let Type::Map(map) = metadata_map else {
            return Err(MmdbError::InvalidMetadata(
                "metadata was not encoded as map",
            ));
        };
        let Some(Type::U32(node_count)) = map.get("node_count") else {
            return Err(MmdbError::InvalidMetadata("does not contain node count"));
        };
        let node_count = *node_count;
        let Some(Type::U16(record_size)) = map.get("record_size") else {
            return Err(MmdbError::InvalidMetadata("does not contain record size"));
        };
        let record_size = *record_size;
        let Some(Type::U16(ip_version)) = map.get("ip_version") else {
            return Err(MmdbError::InvalidMetadata("does not contain ip version"));
        };
        let ip_version = *ip_version;

        Ok(Self {
            node_count,
            record_size,
            ip_version,
        })
    }
}

#[derive(Clone, Debug)]
#[allow(unused)]
pub enum Type {
    Utf8String(String),
    Double(f64),
    Bytes(Vec<u8>),
    U16(u16),
    U32(u32),
    S32(i32),
    U64(u64),
    U128(u128),
    Map(BTreeMap<String, Type>),
    Array(Vec<Type>),
    DataCacheContainer,
    EndMarker,
    Boolean(bool),
    Float(f32),
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        pub fn pretty_print_type(
            typ: &Type,
            indentation: usize,
            f: &mut std::fmt::Formatter<'_>,
        ) -> std::fmt::Result {
            match typ {
                Type::Utf8String(x) => write!(f, "{x}"),
                Type::Double(x) => write!(f, "{x}"),
                Type::Bytes(items) => {
                    write!(f, "[")?;
                    let mut items = items.iter().peekable();
                    while let Some(byte) = items.next() {
                        let last = items.peek().is_none();
                        write!(f, "{byte:0X}")?;
                        if !last {
                            write!(f, ", ")?;
                        }
                    }
                    write!(f, "]")
                }
                Type::U16(x) => write!(f, "{x}"),
                Type::U32(x) => write!(f, "{x}"),
                Type::S32(x) => write!(f, "{x}"),
                Type::U64(x) => write!(f, "{x}"),
                Type::U128(x) => write!(f, "{x}"),
                Type::Map(hash_map) => {
                    let mut known_pretty_names = HashMap::new();
                    known_pretty_names.insert("asn", "ASN");
                    known_pretty_names.insert("country_code", "Country Code");
                    known_pretty_names.insert("domain", "Domain");
                    known_pretty_names.insert("name", "Name");
                    known_pretty_names.insert("network", "Network");
                    known_pretty_names.insert("org", "Organization");

                    let indentation = match indentation {
                        0 => 0,
                        x => x + 1,
                    };

                    if indentation != 0 {
                        // writeln!(f, "{{")?;
                        for _ in 0..indentation {
                            //write!(f, "\t")?;
                        }
                    }

                    let mut hash_map = hash_map.iter().peekable();

                    while let Some((key, value)) = hash_map.next() {
                        let key = match known_pretty_names.get(&key.as_str()) {
                            Some(name) => name,
                            None => key.as_str(),
                        };
                        let last = hash_map.peek().is_none();
                        for _ in 0..indentation {
                            write!(f, " ")?;
                        }
                        write!(f, "{key}: ")?;
                        if matches!(value, Type::Map(_)) {
                            writeln!(f)?;
                        }
                        pretty_print_type(value, indentation + 1, f)?;
                        if !last {
                            writeln!(f, ",")?;
                        }
                    }
                    fmt::Result::Ok(())
                }
                Type::Array(items) => {
                    writeln!(f)?;
                    let mut items = items.iter().peekable();
                    while let Some(value) = items.next() {
                        let last = items.peek().is_none();
                        pretty_print_type(value, indentation, f)?;
                        if !last {
                            write!(f, ", ")?;
                        }
                    }
                    fmt::Result::Ok(())
                }
                Type::DataCacheContainer => write!(f, "[data cache container]"),
                Type::EndMarker => write!(f, "[end marker]"),
                Type::Boolean(x) => write!(f, "{x}"),
                Type::Float(x) => write!(f, "{x}"),
            }
        }
        pretty_print_type(self, 0, f)?;
        std::fmt::Result::Ok(())
    }
}

fn bytes_per_node(record_size: u16) -> Result<u64, MmdbError> {
    match record_size {
        24 => Ok(6),
        28 => Ok(7),
        32 => Ok(8),
        _ => Err(MmdbError::InvalidMetadata("unsupported record_size")),
    }
}

fn read_type<T>(reader: &mut T, metadata: Option<&MmdbMetadata>) -> Result<Type, MmdbError>
where
    T: Read + Seek,
{
    let metadata_control_byte = reader.read_u8()?; // CODE FAILS HERE, overflow?

    let typ = metadata_control_byte >> 5;
    let size_hint = metadata_control_byte & 0x1f;
    // println!("{metadata_control_byte:0b} {typ:0b} {size_hint:0b}");

    let typ = match typ {
        0 => 7u8 + reader.read_u8()?,
        _ => typ,
    };

    let size: u32 = if typ != 1 {
        match size_hint {
            0..=28 => size_hint.into(),
            29 => (29 + reader.read_u8()?).into(),
            30 => (285 + reader.read_u16::<BigEndian>()?).into(),
            31 => 65821 + reader.read_u24::<BigEndian>()?,
            _ => return Err(MmdbError::InvalidData("invalid size field")),
        }
    } else {
        size_hint as u32
    };

    match typ {
        1 => {
            let pointer_size = size >> 3;
            let pointer_value = size & 7;

            match pointer_size {
                0 => {
                    let next_byte = reader.read_u8()?;
                    let next_value: u16 = next_byte as u16 + ((pointer_value as u16) << 8);
                    if let Some(metadata) = metadata {
                        let data_section_offset = bytes_per_node(metadata.record_size)?
                            * metadata.node_count as u64
                            + next_value as u64
                            + 16;
                        let pos = reader.stream_position()?;
                        reader.seek(SeekFrom::Start(data_section_offset))?;
                        let typ = read_type(reader, Some(metadata));
                        reader.seek(SeekFrom::Start(pos))?;
                        typ
                    } else {
                        Err(MmdbError::InvalidData(
                            "pointer addressed before metadata parsed",
                        ))
                    }
                }
                1 => {
                    let mut buf = [0u8; 2];
                    reader.read_exact(&mut buf)?;
                    let value = u16::from_be_bytes(buf);
                    let next_value: u32 = value as u32 + (pointer_value << 16) + 2048;
                    if let Some(metadata) = metadata {
                        let data_section_offset = bytes_per_node(metadata.record_size)?
                            * metadata.node_count as u64
                            + next_value as u64
                            + 16;
                        let pos = reader.stream_position()?;
                        reader.seek(SeekFrom::Start(data_section_offset))?;
                        let typ = read_type(reader, Some(metadata));
                        reader.seek(SeekFrom::Start(pos))?;
                        typ
                    } else {
                        Err(MmdbError::InvalidData(
                            "pointer addressed before metadata parsed",
                        ))
                    }
                }
                2 => {
                    let mut buf = [0u8; 3];
                    reader.read_exact(&mut buf)?;
                    let value = ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32);
                    let next_value: u32 = value + (pointer_value << 24) + 526336;
                    if let Some(metadata) = metadata {
                        let data_section_offset = bytes_per_node(metadata.record_size)?
                            * metadata.node_count as u64
                            + next_value as u64
                            + 16;
                        let pos = reader.stream_position()?;
                        reader.seek(SeekFrom::Start(data_section_offset))?;
                        let typ = read_type(reader, Some(metadata));
                        reader.seek(SeekFrom::Start(pos))?;
                        typ
                    } else {
                        Err(MmdbError::InvalidData(
                            "pointer addressed before metadata parsed",
                        ))
                    }
                }
                3 => {
                    let next_value = reader.read_u32::<BigEndian>()?;
                    if let Some(metadata) = metadata {
                        let data_section_offset = bytes_per_node(metadata.record_size)?
                            * metadata.node_count as u64
                            + next_value as u64
                            + 16;
                        let pos = reader.stream_position()?;
                        reader.seek(SeekFrom::Start(data_section_offset))?;
                        let typ = read_type(reader, Some(metadata));
                        reader.seek(SeekFrom::Start(pos))?;
                        typ
                    } else {
                        Err(MmdbError::InvalidData(
                            "pointer addressed before metadata parsed",
                        ))
                    }
                }
                _ => Err(MmdbError::InvalidData("invalid pointer size")),
            }
        }
        2 => {
            let mut buffer = vec![0; size as usize];
            reader.read_exact(&mut buffer)?;
            let string = String::from_utf8_lossy(&buffer).to_string();
            Ok(Type::Utf8String(string))
        }
        3 => {
            let data = reader.read_f64::<BigEndian>()?;
            Ok(Type::Double(data))
        }
        4 => {
            let mut buffer = vec![0; size as usize];
            reader.read_exact(&mut buffer)?;
            Ok(Type::Bytes(buffer))
        }
        5 => match size {
            0 => Ok(Type::U16(0)),
            _ => Ok(Type::U16(
                reader.read_uint::<BigEndian>(size as usize)?.try_into()?,
            )),
        },
        6 => match size {
            0 => Ok(Type::U32(0)),
            _ => Ok(Type::U32(
                reader.read_uint::<BigEndian>(size as usize)?.try_into()?,
            )),
        },
        9 => match size {
            0 => Ok(Type::U64(0)),
            _ => Ok(Type::U64(reader.read_uint::<BigEndian>(size as usize)?)),
        },
        10 => match size {
            0 => Ok(Type::U128(0)),
            _ => Ok(Type::U128(reader.read_uint128::<BigEndian>(size as usize)?)),
        },
        8 => match size {
            4 => Ok(Type::S32(reader.read_i32::<BigEndian>()?)),
            0..=3 => Ok(Type::S32(
                reader.read_uint::<BigEndian>(size as usize)? as i32
            )),
            _ => Err(MmdbError::InvalidData("bad s32 size")),
        },
        7 => {
            let mut items = BTreeMap::new();
            for _ in 0..size {
                let key = match read_type(reader, metadata)? {
                    Type::Utf8String(key) => key,
                    _ => {
                        return Err(MmdbError::InvalidData("key field for map is not string"));
                    }
                };
                let value = read_type(reader, metadata)?;
                items.insert(key, value);

                // if items.contains_key("node_count")
                //     && items.contains_key("record_size")
                //     && items.contains_key("ip_version")
                // {
                //     // Cut metadata search off short here, as this is the only information we actually need
                //     //break;
                // } else {
                // }
            }
            Ok(Type::Map(items))
        }
        11 => {
            let mut items = Vec::with_capacity(size as usize);
            for _ in 0..size {
                items.push(read_type(reader, metadata)?);
            }
            Ok(Type::Array(items))
        }
        12 => Err(MmdbError::NotImplemented("data cache container")),
        13 => Err(MmdbError::NotImplemented("end marker")),
        14 => match size {
            0 => Ok(Type::Boolean(false)),
            1 => Ok(Type::Boolean(true)),
            _ => Err(MmdbError::InvalidData("invalid boolean")),
        },
        15 => {
            let value = reader.read_f32::<BigEndian>()?;
            Ok(Type::Float(value))
        }
        _ => Err(MmdbError::InvalidData("invalid data type specifier")),
    }
}

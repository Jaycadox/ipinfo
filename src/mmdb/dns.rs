use std::{
    io::Write,
    net::{IpAddr, Ipv4Addr},
};

use byteorder::{BigEndian, WriteBytesExt};

static DNS_SERVER: &str = "1.1.1.1";

#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("Failed to bind UDP socket: {0}")]
    BindFailed(#[source] std::io::Error),
    #[error("Failed to connect to DNS server: {0}")]
    ConnectFailed(#[source] std::io::Error),
    #[error("Failed to write packet: {0}")]
    WriteFailed(#[source] std::io::Error),
    #[error("Failed to send DNS query: {0}")]
    SendFailed(#[source] std::io::Error),
    #[error("Failed to receive DNS response: {0}")]
    RecvFailed(#[source] std::io::Error),
    #[error("Domain part too long")]
    DomainPartTooLong,
    #[error("DNS response too short")]
    ResponseTooShort,
    #[error("DNS error with code: {0}")]
    DnsErrorCode(u8),
    #[error("No DNS record found for domain '{0}'")]
    NoRecordFound(String),
}

pub type DnsResult<T> = Result<T, DnsError>;

pub fn query_dns_for_domain(domain: &str) -> DnsResult<IpAddr> {
    let mut udp = std::net::UdpSocket::bind("0.0.0.0:0").map_err(DnsError::BindFailed)?;
    udp.connect(format!("{DNS_SERVER}:53"))
        .map_err(DnsError::ConnectFailed)?;

    let mut packet = vec![];

    // Header
    // ID, TODO: make random
    packet
        .write_u16::<BigEndian>(0x1234)
        .map_err(DnsError::WriteFailed)?;

    // Flags
    packet
        .write_u16::<BigEndian>(0b0_0000_0_0_1_0_000_0000)
        .map_err(DnsError::WriteFailed)?;

    // COUNTS
    packet
        .write_u16::<BigEndian>(1)
        .map_err(DnsError::WriteFailed)?; // QDCOUNT
    packet
        .write_u16::<BigEndian>(0)
        .map_err(DnsError::WriteFailed)?; // ANCOUNT
    packet
        .write_u16::<BigEndian>(0)
        .map_err(DnsError::WriteFailed)?; // NSCOUNT
    packet
        .write_u16::<BigEndian>(0)
        .map_err(DnsError::WriteFailed)?; // ARCOUNT

    // Question

    // QNAME
    for part in domain.split('.') {
        let part = part.as_bytes();
        let Ok(length) = u8::try_from(part.len()) else {
            return Err(DnsError::DomainPartTooLong);
        };
        packet.write_u8(length).map_err(DnsError::WriteFailed)?;
        packet.write_all(part).map_err(DnsError::WriteFailed)?;
    }
    packet.write_u8(0).map_err(DnsError::WriteFailed)?;

    // QTYPE
    packet
        .write_u16::<BigEndian>(0x0001)
        .map_err(DnsError::WriteFailed)?; // A
    // QCLASS
    packet
        .write_u16::<BigEndian>(0x0001)
        .map_err(DnsError::WriteFailed)?; // IN

    udp.send(&packet).map_err(DnsError::SendFailed)?;
    let mut resp = [0; 512];

    let length = udp.recv(&mut resp).map_err(DnsError::RecvFailed)?;
    let resp = &resp[..length];

    if length < 30 {
        return Err(DnsError::ResponseTooShort);
    }

    let rcode = resp[3] & 0x0F;
    if rcode != 0 {
        return Err(DnsError::DnsErrorCode(rcode));
    }

    let answer_count = ((resp[6] as u16) << 8) | (resp[7] as u16);
    if answer_count == 0 {
        return Err(DnsError::NoRecordFound(domain.to_string()));
    }
    let ip = &resp[length - 4..];
    Ok(IpAddr::V4(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3])))
}

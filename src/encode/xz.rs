use byteorder::{BigEndian, LittleEndian, WriteBytesExt};
use crc::{crc32, Hasher32};
use decode;
use encode::lzma2;
use encode::util;
use std::io;
use std::io::Write;

// TODO: move to some common file for encoder & decoder
const XZ_MAGIC: &[u8] = &[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00];
const XZ_MAGIC_FOOTER: &[u8] = &[0x59, 0x5A];

pub fn encode_stream<R, W>(input: &mut R, output: &mut W) -> io::Result<()>
where
    R: io::BufRead,
    W: io::Write,
{
    // check method = None
    let flags = 0x00;

    // Header
    write_header(output, flags)?;

    // Block
    let (unpadded_size, unpacked_size) = write_block(input, output)?;

    // Index
    let index_size = write_index(output, unpadded_size, unpacked_size)?;

    // Footer
    write_footer(output, flags, index_size)
}

fn write_header<W>(output: &mut W, flags: u16) -> io::Result<()>
where
    W: io::Write,
{
    output.write_all(XZ_MAGIC)?;
    let mut digest = crc32::Digest::new(crc32::IEEE);
    {
        let mut digested = util::HasherWrite::new(output, &mut digest);
        digested.write_u16::<BigEndian>(flags)?;
    }
    let crc32 = digest.sum32();
    output.write_u32::<LittleEndian>(crc32)?;
    Ok(())
}

fn write_footer<W>(output: &mut W, flags: u16, index_size: usize) -> io::Result<()>
where
    W: io::Write,
{
    let mut digest = crc32::Digest::new(crc32::IEEE);
    let mut footer_buf: Vec<u8> = Vec::new();
    {
        let mut digested = util::HasherWrite::new(&mut footer_buf, &mut digest);

        let backward_size = (index_size >> 2) - 1;
        digested.write_u32::<LittleEndian>(backward_size as u32)?;
        digested.write_u16::<BigEndian>(flags)?;
    }
    let crc32 = digest.sum32();
    output.write_u32::<LittleEndian>(crc32)?;
    output.write_all(footer_buf.as_slice())?;

    output.write_all(XZ_MAGIC_FOOTER)?;
    Ok(())
}

fn write_block<R, W>(input: &mut R, output: &mut W) -> io::Result<(usize, usize)>
where
    R: io::BufRead,
    W: io::Write,
{
    let (unpadded_size, unpacked_size) = {
        let mut count_output = util::CountWrite::new(output);

        // Block header
        let mut digest = crc32::Digest::new(crc32::IEEE);
        {
            let mut digested = util::HasherWrite::new(&mut count_output, &mut digest);
            let header_size = 8;
            digested.write_u8((header_size >> 2) as u8)?;
            let flags = 0x00; // 1 filter, no (un)packed size provided
            digested.write_u8(flags)?;
            let filter_id = 0x21; // LZMA2
            digested.write_u8(filter_id)?;
            let size_of_properties = 1;
            digested.write_u8(size_of_properties)?;
            let properties = 22; // TODO
            digested.write_u8(properties)?;
            let padding = [0, 0, 0];
            digested.write(&padding)?;
        }
        let crc32 = digest.sum32();
        count_output.write_u32::<LittleEndian>(crc32)?;

        // Block
        let mut count_input = decode::util::CountBufRead::new(input);
        lzma2::encode_stream(&mut count_input, &mut count_output)?;
        (count_output.count(), count_input.count())
    };
    info!(
        "Unpadded size = {}, unpacked_size = {}",
        unpadded_size, unpacked_size
    );

    let padding_size = ((unpadded_size ^ 0x03) + 1) & 0x03;
    let padding = vec![0; padding_size];
    output.write(padding.as_slice())?;
    // Checksum = None (cf. above)

    Ok((unpadded_size, unpacked_size))
}

fn write_index<W>(output: &mut W, unpadded_size: usize, unpacked_size: usize) -> io::Result<(usize)>
where
    W: io::Write,
{
    let mut count_output = util::CountWrite::new(output);

    let mut digest = crc32::Digest::new(crc32::IEEE);
    {
        let mut digested = util::HasherWrite::new(&mut count_output, &mut digest);
        digested.write_u8(0)?; // No more block
        let num_records = 1;
        write_multibyte(&mut digested, num_records)?;

        write_multibyte(&mut digested, unpadded_size as u64)?;
        write_multibyte(&mut digested, unpacked_size as u64)?;
    }

    // Padding
    let count = count_output.count();
    let padding_size = ((count ^ 0x03) + 1) & 0x03;
    {
        let mut digested = util::HasherWrite::new(&mut count_output, &mut digest);
        let padding = vec![0; padding_size];
        digested.write(padding.as_slice())?;
    }

    let crc32 = digest.sum32();
    count_output.write_u32::<LittleEndian>(crc32)?;

    Ok(count_output.count())
}

fn write_multibyte<W>(output: &mut W, mut value: u64) -> io::Result<()>
where
    W: io::Write,
{
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            output.write_u8(byte)?;
            break;
        } else {
            output.write_u8(0x80 | byte)?;
        }
    }

    Ok(())
}

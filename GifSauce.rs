extern crate lzw;
use lzw::{Encoder, LsbWriter};
use std::env;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Cursor, Error, Read, Seek, SeekFrom, Write};
use std::process::exit;

#[derive(Debug)]
struct GIFHeader {
    signature: [u8; 3], // GIF
    version: [u8; 3],   // 89a
}

#[derive(Debug)]
struct LogicalScreenDescriptor {
    width: u16,
    height: u16,
    packed_field: u8,
    background_color_index: u8,
    pixel_aspect_ratio: u8,
}

#[derive(Debug, Clone)]
struct ColorTable {
    colors: Vec<[u8; 3]>,
}

#[derive(Debug)]
struct GraphicsControlExtension {
    packed_field: u8,
    delay_time: u16,
    transparent_color_index: u8,
}

#[derive(Debug)]
struct CommentExtension {
    comments: Vec<String>,
}

#[derive(Debug)]
struct ApplicationExtension {
    identifier: String,
    authentication_code: String,
    data: Vec<u8>,
}

#[repr(C)] // Ensures the struct has the same memory layout as in C
#[derive(Debug, Clone)]
pub struct PlainTextExtension {
    pub block_size: u8,
    pub text_grid_left_position: u16,
    pub text_grid_top_position: u16,
    pub text_grid_width: u16,
    pub text_grid_height: u16,
    pub character_cell_width: u8,
    pub character_cell_height: u8,
    pub text_foreground_color_index: u8,
    pub text_background_color_index: u8,
    pub plain_text_data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct ImageDescriptor {
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    packed_field: u8,
    local_color_table: Option<ColorTable>, // Include this field
    lzw_minimum_code_size: u8,             // Include this field
    image_data: Vec<u8>,                   // Include the image data field
}

#[derive(Debug)]
struct GIF {
    header: GIFHeader,
    logical_screen_descriptor: LogicalScreenDescriptor,
    global_color_table: Option<ColorTable>,
    graphics_control_extension: Option<GraphicsControlExtension>,
    comment_extensions: Vec<CommentExtension>,
    application_extensions: Vec<ApplicationExtension>,
    plain_text_extensions: Vec<PlainTextExtension>,
    image_descriptors: Vec<ImageDescriptor>,
}

fn track_position<R: Read + Seek>(reader: &mut R, description: &str) -> io::Result<u64> {
    let position = reader.stream_position()?;
    //println!("{} at byte position: {}", description, position);
    Ok(position)
}

fn read_gif_header<R: Read + Seek>(reader: &mut R) -> Result<GIFHeader, Error> {
    track_position(reader, "Start GIF Header")?;
    let mut signature = [0; 3];
    reader.read_exact(&mut signature)?;

    let mut version = [0; 3];
    reader.read_exact(&mut version)?;

    track_position(reader, "End GIF Header")?;
    Ok(GIFHeader { signature, version })
}

fn read_logical_screen_descriptor<R: Read + Seek>(
    reader: &mut R,
) -> Result<LogicalScreenDescriptor, Error> {
    track_position(reader, "Start Logical Screen Descriptor")?;

    let mut width = [0; 2];
    reader.read_exact(&mut width)?;
    let width = u16::from_le_bytes(width);

    let mut height = [0; 2];
    reader.read_exact(&mut height)?;
    let height = u16::from_le_bytes(height);

    let mut packed_field = [0; 1];
    reader.read_exact(&mut packed_field)?;

    let mut background_color_index = [0; 1];
    reader.read_exact(&mut background_color_index)?;

    let mut pixel_aspect_ratio = [0; 1];
    reader.read_exact(&mut pixel_aspect_ratio)?;

    track_position(reader, "End Logical Screen Descriptor")?;
    Ok(LogicalScreenDescriptor {
        width,
        height,
        packed_field: packed_field[0],
        background_color_index: background_color_index[0],
        pixel_aspect_ratio: pixel_aspect_ratio[0],
    })
}

fn read_color_table<R: Read>(reader: &mut R, size: usize) -> Result<ColorTable, Error> {
    let mut colors = Vec::with_capacity(size);
    let mut buffer = [0; 3];

    for _ in 0..size {
        reader.read_exact(&mut buffer)?;
        colors.push(buffer);
    }

    Ok(ColorTable { colors })
}

fn read_graphics_control_extension<R: Read>(
    reader: &mut R,
) -> Result<GraphicsControlExtension, Error> {
    let mut block_size = [0; 1];
    reader.read_exact(&mut block_size)?;

    if block_size[0] != 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid block size for graphics control extension.",
        ));
    }

    let mut packed_field = [0; 1];
    reader.read_exact(&mut packed_field)?;

    let mut delay_time = [0; 2];
    reader.read_exact(&mut delay_time)?;
    let delay_time = u16::from_le_bytes(delay_time);

    let mut transparent_color_index = [0; 1];
    reader.read_exact(&mut transparent_color_index)?;

    let mut terminator = [0; 1];
    reader.read_exact(&mut terminator)?;

    Ok(GraphicsControlExtension {
        packed_field: packed_field[0],
        delay_time,
        transparent_color_index: transparent_color_index[0],
    })
}

fn read_comment_extension<R: Read>(reader: &mut R) -> Result<CommentExtension, Error> {
    let mut comments = Vec::new();

    loop {
        let mut block_size = [0; 1];
        reader.read_exact(&mut block_size)?;
        if block_size[0] == 0 {
            break; // End of comment blocks
        }

        let mut data = vec![0; block_size[0] as usize];
        reader.read_exact(&mut data)?;
        comments.push(String::from_utf8_lossy(&data).into_owned());
    }

    Ok(CommentExtension { comments })
}

fn read_application_extension<R: Read>(reader: &mut R) -> Result<ApplicationExtension, Error> {
    let mut block_size = [0; 1];
    reader.read_exact(&mut block_size)?;

    if block_size[0] != 11 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid application extension block size.",
        ));
    }

    let mut identifier = vec![0; 8];
    reader.read_exact(&mut identifier)?;
    let identifier = String::from_utf8_lossy(&identifier).into_owned();

    let mut authentication_code = vec![0; 3];
    reader.read_exact(&mut authentication_code)?;
    let authentication_code = String::from_utf8_lossy(&authentication_code).into_owned();

    let mut data = Vec::new();

    loop {
        let mut data_block_size = [0; 1];
        reader.read_exact(&mut data_block_size)?;
        if data_block_size[0] == 0 {
            break; // End of application extension data
        }

        let mut block_data = vec![0; data_block_size[0] as usize];
        reader.read_exact(&mut block_data)?;
        data.extend(block_data);
    }

    Ok(ApplicationExtension {
        identifier,
        authentication_code,
        data,
    })
}

fn read_plain_text_extension<R: Read>(reader: &mut R) -> Result<PlainTextExtension, Error> {
    let mut block_size = [0; 1];
    reader.read_exact(&mut block_size)?;
    let block_size = block_size[0];

    let mut text_grid_left_position = [0; 2];
    reader.read_exact(&mut text_grid_left_position)?;
    let text_grid_left_position = u16::from_le_bytes(text_grid_left_position);

    let mut text_grid_top_position = [0; 2];
    reader.read_exact(&mut text_grid_top_position)?;
    let text_grid_top_position = u16::from_le_bytes(text_grid_top_position);

    let mut text_grid_width = [0; 2];
    reader.read_exact(&mut text_grid_width)?;
    let text_grid_width = u16::from_le_bytes(text_grid_width);

    let mut text_grid_height = [0; 2];
    reader.read_exact(&mut text_grid_height)?;
    let text_grid_height = u16::from_le_bytes(text_grid_height);

    let mut character_cell_width = [0; 1];
    reader.read_exact(&mut character_cell_width)?;
    let character_cell_width = character_cell_width[0];

    let mut character_cell_height = [0; 1];
    reader.read_exact(&mut character_cell_height)?;
    let character_cell_height = character_cell_height[0];

    let mut text_foreground_color_index = [0; 1];
    reader.read_exact(&mut text_foreground_color_index)?;
    let text_foreground_color_index = text_foreground_color_index[0];

    let mut text_background_color_index = [0; 1];
    reader.read_exact(&mut text_background_color_index)?;
    let text_background_color_index = text_background_color_index[0];

    let mut plain_text_data = Vec::new();

    loop {
        let mut block_size = [0; 1];
        reader.read_exact(&mut block_size)?;
        if block_size[0] == 0 {
            break; // End of text data
        }

        let mut block_data = vec![0; block_size[0] as usize];
        reader.read_exact(&mut block_data)?;
        plain_text_data.extend(block_data);
    }

    Ok(PlainTextExtension {
        block_size,
        text_grid_left_position,
        text_grid_top_position,
        text_grid_width,
        text_grid_height,
        character_cell_width,
        character_cell_height,
        text_foreground_color_index,
        text_background_color_index,
        plain_text_data,
    })
}

fn read_image_descriptor<R: Read>(reader: &mut R) -> Result<ImageDescriptor, Error> {
    // Read the image separator byte (0x2C)
    let mut separator = [0; 1];
    reader.read_exact(&mut separator)?;

    if separator[0] != 0x2C {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid image descriptor separator.",
        ));
    }

    // Read the image descriptor fields
    let mut left_position = [0; 2];
    reader.read_exact(&mut left_position)?;

    let mut top_position = [0; 2];
    reader.read_exact(&mut top_position)?;

    let mut width = [0; 2];
    reader.read_exact(&mut width)?;

    let mut height = [0; 2];
    reader.read_exact(&mut height)?;

    let mut packed_field = [0; 1];
    reader.read_exact(&mut packed_field)?;

    // Determine the local color table size (if present)
    let local_color_table_size = if (packed_field[0] & 0b10000000) != 0 {
        1 << ((packed_field[0] & 0b00000111) + 1)
    } else {
        0
    };

    // Read the local color table (if present)
    let local_color_table = if local_color_table_size > 0 {
        Some(read_color_table(reader, local_color_table_size as usize)?)
    } else {
        None
    };

    // Read the LZW minimum code size
    let mut lzw_minimum_code_size = [0; 1];
    reader.read_exact(&mut lzw_minimum_code_size)?;

    let lzw_minimum_code_size = lzw_minimum_code_size[0];

    // Read the image data using LZW decompression
    let image_data = read_lzw_data(reader, lzw_minimum_code_size)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    Ok(ImageDescriptor {
        left: u16::from_le_bytes(left_position),
        top: u16::from_le_bytes(top_position),
        width: u16::from_le_bytes(width),
        height: u16::from_le_bytes(height),
        packed_field: packed_field[0],
        local_color_table,
        lzw_minimum_code_size,
        image_data,
    })
}

fn read_lzw_data<R: Read>(reader: &mut R, minimum_code_size: u8) -> Result<Vec<u8>, Error> {
    let mut data = Vec::new();
    let mut block_size = [0; 1];

    // Calculate clear and end-of-information codes based on minimum code size
    let clear_code = 1 << minimum_code_size;
    let end_of_information_code = clear_code + 1;

    // Initialize dictionary with single-byte values
    let mut dictionary: Vec<Vec<u8>> = (0..clear_code).map(|i| vec![i as u8]).collect();
    dictionary.push(vec![]); // Add clear code entry
    dictionary.push(vec![]); // Add EOF code entry
    let mut next_code = clear_code + 2;

    // Initialize variables for reading bit-stream
    let mut bit_buffer: u32 = 0;
    let mut bit_count = 0;
    let mut current_bit_size = minimum_code_size + 1;

    let mut previous_code: Option<u16> = None;

    loop {
        // Read block size
        if let Err(e) = reader.read_exact(&mut block_size) {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(data); // Handle EOF gracefully
            } else {
                return Err(e); // Return other errors
            }
        }

        // Read the data block
        let mut block_data = vec![0; block_size[0] as usize];
        reader.read_exact(&mut block_data)?;

        // Process the block data as a bitstream
        for &byte in &block_data {
            bit_buffer |= (byte as u32) << bit_count;
            bit_count += 8;

            // Extract codes from bit buffer
            while bit_count >= current_bit_size {
                let code = (bit_buffer & ((1 << current_bit_size) - 1)) as u16;
                bit_buffer >>= current_bit_size;
                bit_count -= current_bit_size;

                if code == end_of_information_code {
                    return Ok(data); // End of data
                }

                if code == clear_code {
                    // Reset dictionary and bit size
                    dictionary = (0..clear_code).map(|i| vec![i as u8]).collect();
                    dictionary.push(vec![]);
                    dictionary.push(vec![]);
                    next_code = clear_code + 2;
                    current_bit_size = minimum_code_size + 1;
                    previous_code = None;
                    continue;
                }

                let entry = if code < next_code {
                    dictionary[code as usize].clone()
                } else if let Some(prev_code) = previous_code {
                    let mut new_entry = dictionary[prev_code as usize].clone();
                    new_entry.push(new_entry[0]);
                    new_entry
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid LZW code.",
                    ));
                };

                data.extend(&entry);

                if let Some(prev_code) = previous_code {
                    let mut new_entry = dictionary[prev_code as usize].clone();
                    new_entry.push(entry[0]);
                    if next_code < (1 << 12) {
                        dictionary.push(new_entry);
                        next_code += 1;
                    }

                    // Increase the bit size if necessary
                    if next_code == (1 << current_bit_size) && current_bit_size < 12 {
                        current_bit_size += 1;
                    }
                }

                previous_code = Some(code);
            }
        }
    }
}
fn lzw_compress(data: &[u8], min_code_size: u8) -> Vec<u8> {
    // Create a new cursor that will hold the compressed data
    let mut cursor = Cursor::new(Vec::new());
    let writer = LsbWriter::new(&mut cursor); // Use mutable reference to the cursor

    // Create a new encoder with the specified minimum code size
    let mut encoder = Encoder::new(writer, min_code_size).expect("Failed to create encoder");

    // Encode the data
    encoder
        .encode_bytes(data)
        .expect("Failed to write data to encoder");

    // No need to call finish() or flush() here; just get the underlying Vec<u8>
    // Retrieve the compressed data from the cursor
    drop(encoder); // Ensure the encoder is dropped and the borrow ends
    cursor.into_inner() // Get the underlying Vec<u8>
}

fn parse_gif<R: Read + Seek>(reader: &mut R) -> Result<GIF, Error> {
    let header = read_gif_header(reader)?;
    //println!("Header: {:?}", header);

    let logical_screen_descriptor = read_logical_screen_descriptor(reader)?;
    //println!("Logical Screen Descriptor: {:?}", logical_screen_descriptor);

    // Check global color table size
    let global_color_table_size = if (logical_screen_descriptor.packed_field & 0b111) > 0 {
        1 << ((logical_screen_descriptor.packed_field & 0b111) + 1)
    } else {
        0
    };

    let global_color_table = if global_color_table_size > 0 {
        Some(read_color_table(reader, global_color_table_size)?)
    } else {
        None
    };
    //println!("Global Color Table: {:?}", global_color_table);

    let mut graphics_control_extension = None;
    let mut comment_extensions = Vec::new();
    let mut application_extensions = Vec::new();
    let mut plain_text_extensions = Vec::new();
    let mut image_descriptors = Vec::new();

    let mut plain_text_found = false;
    loop {
        let mut block_indicator = [0; 1];
        match reader.read_exact(&mut block_indicator) {
            Ok(_) => {
                track_position(reader, "Block Indicator")?;
                //ln!("Block Indicator: {:#X}", block_indicator[0]);

                if block_indicator[0] == 0x21 {
                    // Extension Introducer
                    let mut extension_type = [0; 1];
                    reader.read_exact(&mut extension_type)?;

                    match extension_type[0] {
                        0xF9 => {
                            graphics_control_extension =
                                Some(read_graphics_control_extension(reader)?);
                        }
                        0xFE => {
                            comment_extensions.push(read_comment_extension(reader)?);
                        }
                        0xFF => {
                            application_extensions.push(read_application_extension(reader)?);
                        }
                        0x01 => {
                            plain_text_extensions.push(read_plain_text_extension(reader)?);
                            print!(
                                "{}",
                                String::from_utf8_lossy(
                                    plain_text_extensions
                                        .last()
                                        .unwrap()
                                        .plain_text_data
                                        .as_slice()
                                )
                            );
                            plain_text_found = true;
                        }
                        _ => {
                            // Skip unknown extensions
                            loop {
                                let mut block_size = [0; 1];
                                reader.read_exact(&mut block_size)?;
                                if block_size[0] == 0 {
                                    break; // End of this extension block
                                }
                                let mut buffer = vec![0; block_size[0] as usize];
                                reader.read_exact(&mut buffer)?;
                            }
                        }
                    }
                } else if block_indicator[0] == 0x2C {
                    reader.seek(SeekFrom::Current(-1))?;
                    // Image Descriptor
                    let image_descriptor = read_image_descriptor(reader)?;
                    image_descriptors.push(image_descriptor);
                    reader.seek(SeekFrom::Current(1))?;
                } else if block_indicator[0] == 0x3B {
                    // Trailer
                    break;
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid GIF format.",
                    ));
                }
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    break; // Handle EOF gracefully
                }
                return Err(e); // Propagate other errors
            }
        }
    }

    if plain_text_found {
        exit(0);
    }

    Ok(GIF {
        header,
        logical_screen_descriptor,
        global_color_table,
        graphics_control_extension,
        comment_extensions,
        application_extensions,
        plain_text_extensions,
        image_descriptors,
    })
}

// function to reassemble the GIF
fn reassemble_gif<R: Read + Seek>(
    _reader: &mut R,
    output_file: &str,
    gif: &GIF,
) -> Result<(), std::io::Error> {
    let mut writer = BufWriter::new(File::create(output_file)?);

    // 1. Write the GIF header
    writer.write_all(&gif.header.signature)?;
    writer.write_all(&gif.header.version)?;

    // 2. Write the Logical Screen Descriptor
    writer.write_all(&gif.logical_screen_descriptor.width.to_le_bytes())?;
    writer.write_all(&gif.logical_screen_descriptor.height.to_le_bytes())?;
    writer.write_all(&[gif.logical_screen_descriptor.packed_field])?;
    writer.write_all(&[gif.logical_screen_descriptor.background_color_index])?;
    writer.write_all(&[gif.logical_screen_descriptor.pixel_aspect_ratio])?;

    // 3. Write the Global Color Table if present
    if let Some(ref global_color_table) = gif.global_color_table {
        for color in &global_color_table.colors {
            writer.write_all(color)?;
        }
    }

    // 4. Write any Graphics Control Extensions if present
    if let Some(ref graphics_control_extension) = gif.graphics_control_extension {
        writer.write_all(&[0x21, 0xF9, 0x04])?; // Graphics Control Extension introducer
        writer.write_all(&[graphics_control_extension.packed_field])?;
        writer.write_all(&graphics_control_extension.delay_time.to_le_bytes())?;
        writer.write_all(&[graphics_control_extension.transparent_color_index])?;
        writer.write_all(&[0])?; // Block terminator for Graphics Control Extension
    }

    // 5. Write comment extensions
    for comment in &gif.comment_extensions {
        writer.write_all(&[0x21, 0xFE])?; // Comment extension introducer
        let comment_bytes = comment.comments.join("");
        let comment_length = comment_bytes.len() as u8;
        writer.write_all(&[comment_length])?;
        writer.write_all(comment_bytes.as_bytes())?;
        writer.write_all(&[0])?; // Block terminator
    }

    // 6. Write application extensions
    for application in &gif.application_extensions {
        writer.write_all(&[0x21, 0xFF, 0x0B])?; // Application extension introducer
        writer.write_all(application.identifier.as_bytes())?;
        writer.write_all(application.authentication_code.as_bytes())?;

        for chunk in application.data.chunks(255) {
            writer.write_all(&[chunk.len() as u8])?;
            writer.write_all(chunk)?;
        }
        writer.write_all(&[0])?; // Block terminator
    }

    // 7. Write plain text extensions
    let mut first: bool = true;
    let mut index = 0;
    for plain_text in &gif.plain_text_extensions {
        index += 1;
        if first {
            writer.write_all(&[0x21, 0x01, plain_text.block_size])?; // Plain Text Extension introducer
            writer.write_all(&plain_text.text_grid_left_position.to_le_bytes())?;
            writer.write_all(&plain_text.text_grid_top_position.to_le_bytes())?;
            writer.write_all(&plain_text.text_grid_width.to_le_bytes())?;
            writer.write_all(&plain_text.text_grid_height.to_le_bytes())?;
            writer.write_all(&[plain_text.character_cell_width])?;
            writer.write_all(&[plain_text.character_cell_height])?;
            writer.write_all(&[plain_text.text_foreground_color_index])?;
            writer.write_all(&[plain_text.text_background_color_index])?;
            let length = plain_text.plain_text_data.len();
            //println!("Length: {}", length);
            writer.write_all(&[length as u8])?; //255
            writer.write_all(&plain_text.plain_text_data)?; //254
            first = false;
        } else {
            let length = plain_text.plain_text_data.len();
            //println!("Length: {}", length);
            writer.write_all(&[length as u8])?; //255
            writer.write_all(&plain_text.plain_text_data)?; //254
        }
        if gif.plain_text_extensions.len() == index {
            writer.write_all(&[0])?; // Block terminator
        }
    }

    // 8. Write image descriptors
    for image_descriptor in &gif.image_descriptors {
        writer.write_all(&[0x2C])?; // Image separator
        writer.write_all(&image_descriptor.left.to_le_bytes())?;
        writer.write_all(&image_descriptor.top.to_le_bytes())?;
        writer.write_all(&image_descriptor.width.to_le_bytes())?;
        writer.write_all(&image_descriptor.height.to_le_bytes())?;
        writer.write_all(&[image_descriptor.packed_field])?;

        if let Some(ref local_color_table) = image_descriptor.local_color_table {
            for color in &local_color_table.colors {
                writer.write_all(color)?;
            }
        }

        // Write the LZW minimum code size
        writer.write_all(&[image_descriptor.lzw_minimum_code_size])?;

        // Compress and write the image data
        let compressed_data = lzw_compress(
            &image_descriptor.image_data,
            image_descriptor.lzw_minimum_code_size,
        );
        for chunk in compressed_data.chunks(255) {
            writer.write_all(&[chunk.len() as u8])?;
            writer.write_all(chunk)?;
        }
        writer.write_all(&[0])?; // Block terminator
    }

    // 9. Write the GIF trailer
    writer.write_all(&[0x3B])?;

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get input and output file names from command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <input_file> <output_file>", args[0]);
        std::process::exit(1);
    }

    let filename = &args[1];
    let output_file = &args[2];

    // Open the input GIF file
    let file = File::open(filename)?;
    let mut reader = BufReader::new(file);

    // Parse the GIF
    let mut gif = parse_gif(&mut reader)?;

    // Read from stdin and modify the Plain Text Extensions
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("Failed to read from stdin");
        std::process::exit(1);
    }

    let input_chunk = 254;
    // Adjusting to content length.
    loop {
        if (input.len() / input_chunk) > gif.image_descriptors.len() {
            let descriptors_clone = gif.image_descriptors.clone();
            gif.image_descriptors.extend(descriptors_clone);
            println!("Extended image descriptors");
        } else {
            println!("Done extending image descriptors");
            break;
        }
    }
    let mut count_remove = 0;
    let mut new_plain_text_extensions: Vec<PlainTextExtension> = gif
        .image_descriptors
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let new_extended = PlainTextExtension {
                block_size: 12,
                text_grid_left_position: 0,
                text_grid_top_position: 0,
                text_grid_width: 0,
                text_grid_height: 0,
                character_cell_width: 0,
                character_cell_height: 0,
                text_foreground_color_index: 0,
                text_background_color_index: 0,
                plain_text_data: if ((index + 1) * input_chunk) <= input.len() {
                    //println!(
                    //   "length={}, index + 1={}",
                    //    input.len(),
                    //    (index + 1) * input_chunk
                    //);
                    input[index * input_chunk..(index + 1) * input_chunk]
                        .as_bytes()
                        .to_vec()
                } else {
                    //println!("-->index {}", index * input_chunk);
                    let start = index * input_chunk;
                    //println!("Start: {}", start);
                    if start > input.len() {
                        //println!("-->padding");
                        count_remove += 1;
                        vec![0u8; 254] //padding
                    } else {
                        //println!("Start-->bytes: {}", start);
                        input[start..].as_bytes().to_vec()
                    }
                },
            };
            new_extended
        })
        .collect();

    // Remove the padding
    for _ in 0..count_remove {
        gif.image_descriptors.pop();
        new_plain_text_extensions.pop();
    }

    gif.plain_text_extensions = new_plain_text_extensions;

    // Reassemble and write the modified GIF back to a file
    reassemble_gif(&mut reader, output_file, &gif)?;
    println!("GIF reassembled and saved to {}", output_file);

    Ok(())
}

use std::collections::{BTreeMap, HashMap};
use std::fs;

use std::path::PathBuf;

// mod trials;
mod jpeg_utils;

enum Marker {
    SOF0 = 0xFFC0, // Baseline DCT
    SOF3 = 0xFFC3, // Lossless Huffman Encoding
    DHT = 0xFFC4,  // Define Huffman table(s)
    SOI = 0xFFD8,  // Start of image
    EOI = 0xFFD9,  // End of image
    SOS = 0xFFDA,  // Start of scan
    DQT = 0xFFDB,  // Define quantization table(s)
    APP = 0xFFE0,  //Reserved for application segments
    APPn = 0xFFEF, //Reserved for application segments
}

/// Quantization Table, 10918-1, B.2.4.1, P. 39
struct QuantiziationTable {
    p_q: u8,        // Element precision,
    t_q: u8,        // Destinaiton identifier
    q_k: [u16; 64], // Table element
}

struct Component {
    c_: u8,  // Component identifier, 10918-1 P. 36
    h_: u8,  // Horizontal sampling factor
    v_: u8,  // Vertical sampling factor
    t_q: u8, // Quantiziation table destination selector; Not used (0), for lossless
}

struct HeaderParameter {
    c_s: u8, // Scan component selector
    t_d: u8, // DC entropy coding table destination selector
    t_a: u8, // AC entropy coding table destination selector
}

struct ScanHeader {
    // Scan Header, 10918-1, B.2.3, P. 35
    head_params: HashMap<u8, HeaderParameter>,
    s_s: u8, // Start of Spectral selection; predictor selector in lossless
    s_e: u8, // End of Spectral or prediction selection; 0, not used, in lossless
    a_h: u8, // Successive aproximamtion bit position high, 0, not used, in lossless
    a_l_p_t: u8, // Successive approximation bit position low; point transform, Pt, for lossless mode
}

struct FrameHeader {
    // Frame Header, 10918-1, B.2.2, P. 35
    marker: u16,
    p_: u8,  // Sample precision
    y_: u16, // Number of lines
    x_: u16, // Number of samples per line
    components: HashMap<u8, Component>,
}

struct SSSSTable {
    t_c: u8, // Table class – 0 = DC table or lossless table, 1 = AC table
    t_h: u8, // Huffman table destination identifier
    table: HashMap<u32, u8>,
    min_code_length: usize, // number of bits of shorted Huffman code
    max_code_length: usize, // number of bits of longest Huffman code
}

pub struct Jpeg {
    encoded_image: Vec<u8>,
    read_index: usize,
    frame_header: Option<FrameHeader>,
    ssss_tables: HashMap<usize, SSSSTable>,
    // quantization_tables: Option<HashMap<u8, QuantiziationTable>>,
    raw_image: Vec<u32>
}

impl Jpeg {
    pub fn open(path: PathBuf) -> Self {
        let encoded_image = fs::read(path).expect("Unable to read file");
        Self::from_encoded_vec(encoded_image)
    }

    pub fn from_encoded_vec(encoded_image: Vec<u8>) -> Self {
        assert!(jpeg_utils::is_jpeg(&encoded_image[0..2]));

        Self {
            encoded_image: encoded_image,
            read_index: 2,
            frame_header: None,
            ssss_tables: HashMap::new(),
            // quantization_tables: None,
            raw_image: Vec::new(),
        }
    }

    fn decode(&mut self) {
        let encoded_image_len = self.encoded_image.len();

        use Marker::*;
        while self.read_index < encoded_image_len {
            match self.bytes_to_int_two_peeked() {
                marker if marker == SOF3 as u16 => {
                    self.parse_frame_header(marker);
                },
                marker if marker == DHT as u16 => {
                    self.make_ssss_tables();
                },
                marker if marker == SOS as u16 => {
                    self.read_scan();
                    
                },
                marker if marker == EOI as u16 => break,
                marker if marker > 0xFF00 => panic!("Oops, that marker hasn't been implimented yet!"),
                _ => self.read_index += 1,
            }
        }
    }

    fn read_scan(&mut self) {
        self.found_marker();
        let scan_header = jpeg_utils::parse_scan_header(self);
        self.decode_image(scan_header);
    }

    /// TODO: THIS SEEMS TO BE WEHRE I'VE LEFT OFF
    /// 10918-1, H.2, P. 136 & H.1, P. 132
    fn decode_image(&mut self, scan_header: ScanHeader) {
        let image_bits = self.get_image_data_without_stuffed_zero_bytes();
        let mut image_bits = image_bits.iter();

        let frame_header = self.frame_header.as_ref().unwrap();
        let width = frame_header.x_ as usize;
        let height = frame_header.y_ as usize;

        let component_count = frame_header.components.len();
        self.raw_image = Vec::with_capacity(width * height * component_count);

        while width * height * component_count > self.raw_image.len() {
            let component = self.raw_image.len() % component_count;
            let p_x = jpeg_utils::make_prediciton(
                &self.raw_image,
                component_count,
                width,
                frame_header.p_,
                scan_header.a_h,
                scan_header.s_s,
            );
            let pixel_delta = jpeg_utils::get_huffmaned_value(&self.ssss_tables[&component], &mut image_bits);
            self.raw_image.push(((p_x as i32 + pixel_delta) & ((1 << frame_header.p_) - 1)) as u32);
        }
    }

    // ToDo: this is hacky
    fn get_image_data_without_stuffed_zero_bytes(&mut self) -> Vec<u8> {
        // See JPG document 10918-1 P33 B.1.1.5 Note 2
        let mut image_data: Vec<u8> = Vec::with_capacity(self.encoded_image.len());
        let mut this_byte: u8 = self.byte_to_int_one_consumed();
        let mut next_byte: u8 = self.byte_to_int_one_consumed();
        let mut i = 0;
        loop {
            if this_byte < 0xFF {
                // if the current element is less then 0xFF the proceide as usual
                image_data.push(this_byte);
                i += 1;
                this_byte = next_byte;
                match self.encoded_image.get(self.read_index) {
                    Some(n) => {
                        next_byte = *n;
                        self.read_index += 1;
                    },
                    None => break
                }
            } else if next_byte == 0 {
                // given that the current element is 0xFF
                // if the next element is zero then
                // this element should be read and the next is a "zero byte"
                // which was added to avoid confusion with markers and should be discarded
                // ToDo: what if there are consecutive 0xFF?
                image_data.push(this_byte);
                i += 1;
                match self.encoded_image.get(self.read_index) {
                    Some(n) => {
                        this_byte = *n;
                        self.read_index += 1;
                    },
                    None => break
                }
                match self.encoded_image.get(self.read_index) {
                    Some(n) => {
                        next_byte = *n;
                        self.read_index += 1;
                    },
                    None => {}
                }
            } else {
                // Hit the end of the section
                break;
            }
        }

        if this_byte == 0xFF && 0 < next_byte && next_byte < 0xFF {
            self.read_index -= 2;
        }
    
        let mut bits: Vec<u8> = Vec::with_capacity(i * 8);

        for i in image_data.iter() {
            bits.push((i >> 7) & 1);
            bits.push((i >> 6) & 1);
            bits.push((i >> 5) & 1);
            bits.push((i >> 4) & 1);
            bits.push((i >> 3) & 1);
            bits.push((i >> 2) & 1);
            bits.push((i >> 1) & 1);
            bits.push(i & 1);
        }
    
        bits
    }

    fn make_ssss_tables(&mut self) {
        self.found_marker();

        // since I'm returning stuff, should this go in the utils file and then just pass in &mut self
        let (t_c, t_h, code_lengths) = self.parse_huffman_info();

        let (table, min_code_length, max_code_length) = jpeg_utils::make_ssss_table(code_lengths);

        let ssss_table = SSSSTable {
            t_c,
            t_h,
            table,
            min_code_length,
            max_code_length,
        };

        self.ssss_tables.insert(ssss_table.t_h as usize, ssss_table);
    }

    fn parse_huffman_info(&mut self) -> (u8, u8, [[Option<u8>; 16]; 16]) {
        let _l_h: u16 = self.bytes_to_int_two_consumed();
        let t_c_h: u8 = self.byte_to_int_one_consumed();
        let t_c: u8 = t_c_h >> 4;
        let t_h: u8 = t_c_h & 0xF;
        let mut code_lengths: [[Option<u8>; 16]; 16] = [[None; 16]; 16];
        let mut lengths: BTreeMap<u8, u8> = BTreeMap::new();
        for code_length_index in 0..16 {
            let l_i: u8 = self.byte_to_int_one_consumed();
            if l_i > 0 {
                lengths.insert(code_length_index, l_i);
            }
        }
        for (code_length_index, l_i) in lengths.iter() {
            for i in 0..*l_i {
                code_lengths[*code_length_index as usize][i as usize] =
                    Some(self.byte_to_int_one_consumed());
            }
        }
    
        (t_c, t_h, code_lengths)
    }

    fn parse_frame_header(&mut self, marker: u16) {
        // See JPG document 10918-1 P33 B.1.1.5 Note 2
        self.found_marker();
        let _l_f: u16 = self.bytes_to_int_two_consumed();
        let p_: u8 = self.byte_to_int_one_consumed();
        let y_: u16 = self.bytes_to_int_two_consumed();
        let x_: u16 = self.bytes_to_int_two_consumed();
        let n_f: usize = self.byte_to_int_one_consumed() as usize;
        let mut components: HashMap<u8, Component> = HashMap::new();
        for _ in 0..n_f as usize {
            let c_: u8 = self.byte_to_int_one_consumed();
            let h_v: u8 = self.byte_to_int_one_consumed();
            let t_q: u8 = self.byte_to_int_one_consumed();
            components.insert(
                c_,
                Component {
                    c_,
                    h_: h_v >> 4,
                    v_: h_v & 0xF,
                    t_q,
                },
            );
        }
    
        self.frame_header = Some(FrameHeader {
            marker,
            p_,
            y_,
            x_,
            components,
        })
    }

    fn found_marker(&mut self) {
        self.read_index += 2;
    }

    fn bytes_to_int_two_consumed(&mut self) -> u16 {
        let answer = self.bytes_to_int_two_peeked();
        self.read_index += 2;
        answer
    }
    
    fn bytes_to_int_two_peeked(&self) -> u16 {
        u16::from_be_bytes([self.encoded_image[self.read_index], self.encoded_image[self.read_index + 1]])
    }

    fn byte_to_int_one_consumed(&mut self) -> u8 {
        let answer = self.encoded_image[self.read_index];
        self.read_index += 1;
        answer
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    // #[test]
    // fn test_functional() {
    //     use std::path::Path;
    //     use std::fs::File;
    //     extern crate image;
        
    //     let mut path = env::current_dir().unwrap();
    //     path.push("tests/common/F-18.ljpg");

    //     let mut img = Jpeg::open(path);
    //     img.decode();
        
    //     let width = img.frame_header.as_ref().unwrap().x_;
    //     let height = img.frame_header.as_ref().unwrap().y_;

    //     let mut buffer: Vec<u8> = Vec::with_capacity(img.raw_image.len());
    //     img.raw_image.iter().for_each(|r| buffer.push(*r as u8));

    //     image::save_buffer(&Path::new("image.jpg"), &buffer, width as u32, height as u32, image::ColorType::Rgb8);
        
    //     assert!(img.encoded_image.len() == 107760);
    //     assert_eq!(img.read_index, 2);
    // }

    #[test]
    fn get_image_data_without_stuffed_zero_bytes_good_reguar_number_then_marker() {
        let encoded_image: Vec<u8> = Vec::from([0x00, 0xFE, 0x00, 0xFF, 0x00, 0x05, 0xFF, 0xDA]);
        let expected_bits: Vec<u8> = Vec::from([
            0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1,
            1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 1,
        ]);

        let mut image = Jpeg{
            encoded_image,
            read_index: 0,
            frame_header: None,
            ssss_tables: HashMap::new(),
            raw_image: Vec::new(),
        };

        let actual_bits = image.get_image_data_without_stuffed_zero_bytes();

        assert_eq!(actual_bits, expected_bits);
        assert_eq!(actual_bits.len(), 40);
        assert_eq!(image.encoded_image[image.read_index], 0xFF);
        assert_eq!(image.encoded_image[image.read_index + 1], 0xDA);
    }

    #[test]
    fn get_image_data_without_stuffed_zero_bytes_good_padding_then_marker() {
        let encoded_image: Vec<u8> = Vec::from([0x00, 0xFE, 0x00, 0xFF, 0x00, 0xFF, 0xDA]);
        let expected_bits: Vec<u8> = Vec::from([
            0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1,
            1, 1, 1,
        ]);

        let mut image = Jpeg{
            encoded_image,
            read_index: 0,
            frame_header: None,
            ssss_tables: HashMap::new(),
            raw_image: Vec::new(),
        };

        let actual_bits = image.get_image_data_without_stuffed_zero_bytes();

        assert_eq!(actual_bits, expected_bits);
        assert_eq!(actual_bits.len(), 32);
        assert_eq!(image.encoded_image[image.read_index], 0xFF);
        assert_eq!(image.encoded_image[image.read_index + 1], 0xDA);
    }

    #[test]
    fn get_image_data_without_stuffed_zero_bytes_good_reguar_number_with_no_marker() {
        let encoded_image: Vec<u8> = Vec::from([0x00, 0xFE, 0x00, 0xFF, 0x00, 0x05]);
        let expected_bits: Vec<u8> = Vec::from([
            0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1,
            1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 1,
        ]);

        let mut image = Jpeg{
            encoded_image,
            read_index: 0,
            frame_header: None,
            ssss_tables: HashMap::new(),
            raw_image: Vec::new(),
        };

        let actual_bits = image.get_image_data_without_stuffed_zero_bytes();

        assert_eq!(actual_bits, expected_bits);
        assert_eq!(actual_bits.len(), 40);
        assert_eq!(image.read_index, 6);
    }

    #[test]
    fn get_image_data_without_stuffed_zero_bytes_good_padding_with_no_marker() {
        let encoded_image: Vec<u8> = Vec::from([0x00, 0xFE, 0x00, 0xFF, 0x00]);
        let expected_bits: Vec<u8> = Vec::from([
            0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1,
            1, 1, 1,
        ]);

        let mut image = Jpeg{
            encoded_image,
            read_index: 0,
            frame_header: None,
            ssss_tables: HashMap::new(),
            raw_image: Vec::new(),
        };

        let actual_bits = image.get_image_data_without_stuffed_zero_bytes();

        assert_eq!(actual_bits, expected_bits);
        assert_eq!(actual_bits.len(), 32);
        assert_eq!(image.read_index, 5);
    }

    #[test]
    fn test_open() {
        let mut path = env::current_dir().unwrap();
        path.push("tests/common/F-18.ljpg");

        let image = Jpeg::open(path);
        
        assert!(image.encoded_image.len() == 107760);
        assert_eq!(image.read_index, 2);
        // assert!(image.raw_image == None);
    }

    #[test]
    fn test_from_encoded_vec() {
        let mut path = env::current_dir().unwrap();
        path.push("tests/common/F-18.ljpg");
        let encoded_image = fs::read(path).expect("Unable to read file");

        let image = Jpeg::from_encoded_vec(encoded_image);
        
        assert!(image.encoded_image.len() == 107760);
        assert_eq!(image.read_index, 2);
        // assert!(image.raw_image == None);
    }


    #[test]
    fn parse_frame_header_good() {
        let mut path = env::current_dir().unwrap();
        path.push("tests/common/F-18.ljpg");
        let mut image = Jpeg::open(path);
        image.read_index = 2;

        image.parse_frame_header(Marker::SOF3 as u16);


        assert_eq!(image.frame_header.as_ref().unwrap().marker, 0xFFC3);
        assert_eq!(image.frame_header.as_ref().unwrap().p_, 0x08);
        assert_eq!(image.frame_header.as_ref().unwrap().y_, 0x00F0);
        assert_eq!(image.frame_header.as_ref().unwrap().x_, 0x0140);
        assert_eq!(image.frame_header.as_ref().unwrap().components.len(), 3);
        assert_eq!(image.frame_header.as_ref().unwrap().components.get(&0).unwrap().h_, 1);
        assert_eq!(image.frame_header.as_ref().unwrap().components.get(&0).unwrap().v_, 1);
        assert_eq!(image.frame_header.as_ref().unwrap().components.get(&0).unwrap().t_q, 0);
        assert_eq!(image.frame_header.as_ref().unwrap().components.get(&1).unwrap().h_, 1);
        assert_eq!(image.frame_header.as_ref().unwrap().components.get(&1).unwrap().v_, 1);
        assert_eq!(image.frame_header.as_ref().unwrap().components.get(&1).unwrap().t_q, 0);
        assert_eq!(image.frame_header.as_ref().unwrap().components.get(&2).unwrap().h_, 1);
        assert_eq!(image.frame_header.as_ref().unwrap().components.get(&2).unwrap().v_, 1);
        assert_eq!(image.frame_header.as_ref().unwrap().components.get(&2).unwrap().t_q, 0);
        assert_eq!(image.read_index, 21);
        assert_eq!(image.bytes_to_int_two_consumed(), 0xFFC4);
    }

    #[test]
    fn test_bytes_to_int_two_consumed() {
        let mut image = Jpeg {
            encoded_image: vec![5, 6],
            read_index: 0,
            ssss_tables: HashMap::new(),
            frame_header: None,
            // quantization_tables: None,
            raw_image: Vec::new(),
        };

        assert_eq!(image.bytes_to_int_two_consumed(), 1286);
        assert_eq!(image.read_index, 2)
    }
    
    #[test]
    fn test_bytes_to_int_two_peeked() {
        let image = Jpeg {
            encoded_image: vec![5, 6],
            read_index: 0,
            ssss_tables: HashMap::new(),
            frame_header: None,
            // quantization_tables: None,
            raw_image: Vec::new(),
        };

        assert_eq!(image.bytes_to_int_two_peeked(), 1286);
        assert_eq!(image.read_index, 0)
    }

    #[test]
    fn test_byte_to_int_one_consumed() {
        let mut image = Jpeg {
            encoded_image: vec![5, 6],
            read_index: 0,
            ssss_tables: HashMap::new(),
            frame_header: None,
            // quantization_tables: None,
            raw_image: Vec::new(),
        };

        assert_eq!(image.byte_to_int_one_consumed(), 5);
        assert_eq!(image.read_index, 1)
    }
}

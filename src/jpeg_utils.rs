use std::collections::{BTreeMap, HashMap};
use std::slice::Iter;

use crate::{HeaderParameter, Jpeg, ScanHeader, SSSSTable};

pub(crate) struct ContextContext<'a> {
    pub(crate) component: usize,
    pub(crate) x_position: usize,
    pub(crate) y_position: usize,
    pub(crate) width: usize,
    pub(crate) component_count: usize,
    pub(crate) p_t: u8,
    pub(crate) p_: u8, // Sample precision
    pub(crate) img: &'a Vec<u32>,
}

impl ContextContext<'_> {
    pub(crate) fn r_a(&self) -> i32 {
        self.img[(self.x_position - 1) * self.component_count
            + self.y_position * self.width * self.component_count
            + self.component] as i32
    }
    pub(crate) fn r_b(&self) -> i32 {
        self.img[self.x_position * self.component_count
            + (self.y_position - 1) * self.width * self.component_count
            + self.component] as i32
    }
    pub(crate) fn r_c(&self) -> i32 {
        self.img[(self.x_position - 1) * self.component_count
            + (self.y_position - 1) * self.width * self.component_count
            + self.component] as i32
    }
    pub(crate) fn r_ix(&self) -> i32 {
        1 << (self.p_ - self.p_t - 1) as i32
    }
}

pub(crate) fn get_huffmaned_value(
    ssss_table: &SSSSTable,
    image_bits: &mut Iter<u8>,
) -> i32 {
    let mut ssss: u8 = 0xFF;
    let mut guess: u32 = 1;

    for _ in 0..ssss_table.min_code_length - 1 {
        guess = (guess << 1) | (*image_bits.next().unwrap() as u32);
    }

    // TODO: seems like it should be min_code..max_code, or something like that
    for _ in 0..ssss_table.max_code_length {
        guess = (guess << 1) | (*image_bits.next().unwrap() as u32);
        if ssss_table.table.contains_key(&guess) {
            ssss = ssss_table.table[&guess];
            break;
        }
    }

    match ssss {
        0xFF => {
            // if no code is matched return a zero, this was said to be the safest somewhere
            // TODO: should if break or be error resistant? also goes for down below
            // panic!("No matching Huffman code was found for a lossless tile jpeg.")
            // warnings.warn('A Huffman coding error was found in a lossless jpeg in a dng; it may'
            //               + ' have been resolved, there may be corrupted data')
            panic!("bad huffmaned code!");
        }
        16 => 32768,
        _ => {
            let mut pixel_diff: u16 = 0;
            if ssss > 0 {
                let first_bit = *image_bits.next().unwrap();
                // TODO: seems like the "(pixel_diff << 1) |" is unnecessary
                pixel_diff = (pixel_diff << 1) | (first_bit as u16);
                // step thru the remainder of the ssss number of bits to get the coded number
                for _ in 0..ssss - 1 {
                    pixel_diff = (pixel_diff << 1) | (*image_bits.next().unwrap() as u16);
                }
                // if the first read bit is 0 the number is negative and has to be calculated
                if first_bit == 0 {
                    -(((1 << ssss) - (pixel_diff + 1)) as i32)
                } else {
                    pixel_diff as i32
                }
            } else {
                0
            }
        }
    }
}

pub(crate) fn make_prediciton(
    raw_image: &Vec<u32>,
    // idx: usize,
    component_count: usize,
    width: usize,
    p_: u8,
    p_t: u8,
    predictor: u8,
) -> u32 {
    let idx = raw_image.len();
    let component = idx % component_count;
    let context = ContextContext {
        component,
        x_position: (idx / component_count) % width,
        y_position: (idx / component_count) / width,
        width,
        component_count,
        p_t,
        p_,
        img: raw_image,
    };
    predict(context, predictor)
}

fn predict(context: ContextContext, mut predictor: u8) -> u32 {
    if context.x_position == 0 {
        if context.y_position == 0 {
            predictor = 8;
        } else {
            predictor = 2;
        }
    } else if context.y_position == 0 {
        predictor = 1;
    }

    match predictor {
        0 => 0,
        1 => context.r_a() as u32,
        2 => context.r_b() as u32,
        3 => context.r_c() as u32,
        4 => (context.r_a() + context.r_b() - context.r_c()) as u32,
        5 => (context.r_a() + ((context.r_b() - context.r_c()) >> 1)) as u32,
        6 => (context.r_b() + ((context.r_a() - context.r_c()) >> 1)) as u32,
        7 => ((context.r_a() + context.r_b()) / 2) as u32,
        _ => 2u32.pow((context.p_ - context.p_t - 1) as u32),
    }
}

pub(crate) fn parse_scan_header(image: &mut Jpeg) -> ScanHeader {
    let _l_s: u16 = image.bytes_to_int_two_consumed();
    let n_s: usize = image.byte_to_int_one_consumed() as usize;
    let mut head_params: HashMap<u8, HeaderParameter> = HashMap::new();
    for _ in 0..n_s {
        let c_s: u8 = image.byte_to_int_one_consumed();
        let t_d_a: u8 = image.byte_to_int_one_consumed();
        head_params.insert(
            c_s,
            HeaderParameter {
                c_s,
                t_d: t_d_a >> 4,
                t_a: t_d_a & 0xF,
            },
        );
    }
    let s_s: u8 = image.byte_to_int_one_consumed();
    let s_e: u8 = image.byte_to_int_one_consumed();
    let a_h_l: u8 = image.byte_to_int_one_consumed();
    let a_h: u8 = a_h_l >> 4;
    let a_l_p_t: u8 = a_h_l & 0xF;

    ScanHeader {
        head_params,
        s_s,
        s_e,
        a_h,
        a_l_p_t,
    }
}

/// TODO: this algerythom presumably doesn't work for all possible tables
// fn make_ssss_table(code_lengths: [[u8; 16]; 16]) -> (HashMap<u32, u8>, usize, usize) {
pub(crate) fn make_ssss_table(code_lengths: [[Option<u8>; 16]; 16]) -> (HashMap<u32, u8>, usize, usize) {
    // https://www.youtube.com/watch?v=dM6us854Jk0

    // Codes start towards the top left of the tree
    // As you move to the right, trailing 1s are added
    // As you move down, bits are added
    // So the left most bit represents the top row

    // 0xFF, 0xFF, 0xFF, 0xFF
    // 0x0,  0x1,  0x2,  0xFF
    // 0x3,  0xFF, 0xFF, 0xFF
    // 0x4,  0xFF, 0xFF, 0xFF

    //                                     /   \
    // Index 0, Code Length 1, Codes:     0     1
    // Value:                            NA    NA
    //                                   /\    / \
    // Index 1, Code Length 2, Codes:  00 01  10 11
    // Value:                           0  1   2 NA
    //                                           / \
    // Index 2, Code Length 3, Codes:         110  111
    // Value:                                   3   NA
    //                                              / \
    // Index 3: Code Length 4, Codes:            1110  1111
    // Values:                                      4   NA
    // NOTE: padded/leading 1 not shown so all above codes would be stored with an
    // additional 1 in front of it.

    // let mut code: u32 = 1;
    // let mut table: HashMap<u32, u8> = HashMap::new();

    // // Iterate over all of the rows of the tree
    // for bits_in_code_minus_1 in 0..16 {
    //     // for each row, add another bit
    //     code = code << 1;
    //     // if there are no codes with that number of bits go to the next row
    //     if code_lengths[bits_in_code_minus_1][0].is_none() {
    //         continue;
    //     }

    //     // let mut values_w_n_bits: usize = 0;
    //     // let values: std::slice::Iter<Option<u8>> = code_lengths[bits_in_code_minus_1].iter();
    //     for (i, value) in code_lengths[bits_in_code_minus_1].iter().enumerate() {
    //         if value.is_none() {
    //             break;
    //         }
    //         if i > 0 {
    //             let mut only_removed_ones = true;
    //             // shouldn't need number_of_... it's just there to prevent errors
    //             while only_removed_ones && number_of_used_bits(&code) > 0 {
    //                 only_removed_ones = code & 1 == 1;
    //                 code = code >> 1;
    //             }
    //             code = (code << 1) + 1;

    //             while number_of_used_bits(&code) < bits_in_code_minus_1 + 2 {
    //                 code = code << 1;
    //             }
    //             // code = code << (bits_in_code_minus_1 + 1 - (number_of_used_bits(&code) - 1));
    //         }
    //         table.insert(code, value.unwrap());
    //     }
    // }

    // storing the huffman code in the bits of a u32
    // the code is preceided by a 1 so there can be leading zeros
    let mut code: u32 = 1;
    let mut table: HashMap<u32, u8> = HashMap::new();
    for (index, row) in code_lengths.iter().enumerate() {
        // the code lengths (number of bytes) are stored in a HashMap that was initized with 0xFF
        // and the codes only go up to 16,
        // so if the first cell has 0xFF then there are no codes with a length
        // equal to that row's index
        // so remove the rows that still have the initial value, 0xFF
        // since, as previously discussed, there aren't any codes of that length

        // probably slower than the following but it's cleaner so... if row[0].is_some() {
        // filter out the values that have 0xFF since those are initial values
        // and don't have a valid code length
        let values = row.iter().filter_map(|x| *x).collect::<Vec<u8>>();
        if !values.is_empty() {
            // for each code lengh start with the 0th code of that length
            let mut values_w_n_bits: usize = 0;
            // once all codes of a length have been processed,
            // move on
            while values_w_n_bits <= values.len() {
                // Shift the padded/leading 1 so that the code's the right length
                // index + 1 is the desired code length since index is base 0 so one less then the code length
                // number_of_used_bits(&code) - 1 is the present code length since the leading/padded one takes up a bit
                // the desired code length - the present code length is the amount it must grow to achieve the desired length
                code = code << (index + 1 - (number_of_used_bits(&code) - 1));
                // While the first code of a langth "automatically" works,
                // additionl codes of a length must have bits flipped
                if values_w_n_bits > 0 {
                    // Remove bits (move up the tree) until you remove a 0
                    // (so you can move to the right branch from the left)
                    // Or until you hit the top (again, so you can move to the right branch)
                    loop {
                        let removed: u32 = code & 1;
                        code >>= 1;
                        // if !(removed == 1 && number_of_used_bits(&code) > 1) {
                        if removed == 0 || number_of_used_bits(&code) <= 1 {
                            break;
                        }
                    }
                    // Move down and to the right one node along the tree
                    code = (code << 1) + 1;
                    // Extend the code until it's appropreately long
                    code = code << ((index + 1) - (number_of_used_bits(&code) - 1));
                }
                if values.len() > values_w_n_bits {
                    table.insert(code, values[values_w_n_bits]);
                }
                values_w_n_bits += 1;
            }
        }
    }

    let mut min_code_length: usize = 100;
    let mut max_code_length: usize = 0;

    for v in table.keys() {
        let length = number_of_used_bits(v) - 1;
        if length < min_code_length {
            min_code_length = length;
        }
        if length > max_code_length {
            max_code_length = length;
        }
    }

    (table, min_code_length, max_code_length)
}

pub(crate) fn is_jpeg(bytes: &[u8]) -> bool {
    if u16::from_be_bytes([bytes[0], bytes[1]]) == 0xFFD8 as u16 {
        true
    } else {
        false
    }
}

pub(crate) fn number_of_used_bits(numb: &u32) -> usize {
    (32 - numb.leading_zeros()) as usize
}

#[cfg(test)]
mod tests {
    // extern crate test;
    #[test]
    fn get_huffmaned_value_0_bits() {
        let ssss_table = SSSSTable {
            t_c: 0,
            t_h: 0,
            table: HashMap::from([
                (4, 0),
                (30, 4),
                (6, 2),
                (126, 6),
                (254, 7),
                (510, 8),
                (14, 3),
                (5, 1),
                (62, 5),
            ]),
            min_code_length: 2,
            max_code_length: 8,
        };
        let image_bits: Vec<u8> = Vec::from([0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let pixel_diff = get_huffmaned_value(&ssss_table, &mut image_bits.iter());
        assert_eq!(pixel_diff, 0);
    }

    #[test]
    fn get_huffmaned_value_1_bit() {
        let ssss_table = SSSSTable {
            t_c: 0,
            t_h: 0,
            table: HashMap::from([
                (4, 0),
                (30, 4),
                (6, 2),
                (126, 6),
                (254, 7),
                (510, 8),
                (14, 3),
                (5, 1),
                (62, 5),
            ]),
            min_code_length: 2,
            max_code_length: 8,
        };
        let image_bits: Vec<u8> = Vec::from([0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let pixel_diff = get_huffmaned_value(&ssss_table, &mut image_bits.iter());
        assert_eq!(pixel_diff, 1);
    }

    #[test]
    fn get_huffmaned_value_1_bit_neg() {
        let ssss_table = SSSSTable {
            t_c: 0,
            t_h: 0,
            table: HashMap::from([
                (4, 0),
                (30, 4),
                (6, 2),
                (126, 6),
                (254, 7),
                (510, 8),
                (14, 3),
                (5, 1),
                (62, 5),
            ]),
            min_code_length: 2,
            max_code_length: 8,
        };
        let image_bits: Vec<u8> = Vec::from([0, 1, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let pixel_diff = get_huffmaned_value(&ssss_table, &mut image_bits.iter());
        assert_eq!(pixel_diff, -1);
    }

    #[test]
    fn get_huffmaned_value_2_bits() {
        let ssss_table = SSSSTable {
            t_c: 0,
            t_h: 0,
            table: HashMap::from([
                (4, 0),
                (30, 4),
                (6, 2),
                (126, 6),
                (254, 7),
                (510, 8),
                (14, 3),
                (5, 1),
                (62, 5),
            ]),
            min_code_length: 2,
            max_code_length: 8,
        };
        let image_bits: Vec<u8> = Vec::from([1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let pixel_diff = get_huffmaned_value(&ssss_table, &mut image_bits.iter());
        assert_eq!(pixel_diff, 3);
    }

    #[test]
    fn get_huffmaned_value_2_bits_neg() {
        let ssss_table = SSSSTable {
            t_c: 0,
            t_h: 0,
            table: HashMap::from([
                (4, 0),
                (30, 4),
                (6, 2),
                (126, 6),
                (254, 7),
                (510, 8),
                (14, 3),
                (5, 1),
                (62, 5),
            ]),
            min_code_length: 2,
            max_code_length: 8,
        };
        let image_bits: Vec<u8> = Vec::from([1, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let pixel_diff = get_huffmaned_value(&ssss_table, &mut image_bits.iter());
        assert_eq!(pixel_diff, -2);
    }

    #[test]
    fn get_huffmaned_value_16_bits() {
        let ssss_table = SSSSTable {
            t_c: 0,
            t_h: 0,
            table: HashMap::from([
                (4, 0),
                (30, 4),
                (6, 16),
                (126, 6),
                (254, 7),
                (510, 8),
                (14, 3),
                (5, 1),
                (62, 5),
            ]),
            min_code_length: 2,
            max_code_length: 16,
        };
        let image_bits: Vec<u8> = Vec::from([1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let pixel_diff = get_huffmaned_value(&ssss_table, &mut image_bits.iter());
        assert_eq!(pixel_diff, 32768);
    }

    // #[test]
    // // #[should_panic(expected = "No matching Huffman code was found for a lossless tile jpeg.")]
    // fn get_huffmaned_value_panic() {
    //     let ssss_table = SSSSTable {
    //         t_c: 0,
    //         t_h: 0,
    //         table: HashMap::from([
    //             (4, 0),
    //             (30, 4),
    //             (6, 16),
    //             (126, 6),
    //             (254, 7),
    //             (510, 8),
    //             (14, 3),
    //             (5, 1),
    //             (62, 5),
    //         ]),
    //         min_code_length: 2,
    //         max_code_length: 8,
    //     };
    //     let image_bits: Vec<u8> = Vec::from([
    //         1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 1,
    //     ]);
    //     let pixel_diff = get_huffmaned_value(&ssss_table, &mut image_bits.iter());
    //     println!("{:?}", pixel_diff);
    //     assert_eq!(
    //         pixel_diff.unwrap_err().to_string(),
    //         HuffmanDecodingError::Default.to_string()
    //     );
    // }

    #[test]
    fn make_ssss_tables_good() {
        let code_lengths = [
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                Some(0),
                Some(1),
                Some(2),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            [
                Some(3),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            [
                Some(4),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            [
                Some(5),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            [
                Some(6),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            [
                Some(7),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            [
                Some(8),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
        ];

        let expected = HashMap::from([
            (4, 0),
            (30, 4),
            (6, 2),
            (126, 6),
            (254, 7),
            (510, 8),
            (14, 3),
            (5, 1),
            (62, 5),
        ]);

        let (tables, min_code_length, max_code_length) = make_ssss_table(code_lengths);

        assert_eq!(tables, expected);
        assert_eq!(min_code_length, 2);
        assert_eq!(max_code_length, 8);
    }

    #[test]
    fn make_ssss_tables_good2() {
        let code_lengths = [
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                Some(0),
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
        ];

        let expected = HashMap::from([(8, 0), (9, 1), (10, 2), (11, 3), (12, 4), (13, 5), (14, 6)]);

        let (tables, min_code_length, max_code_length) = make_ssss_table(code_lengths);

        assert_eq!(tables, expected);
        assert_eq!(min_code_length, 3);
        assert_eq!(max_code_length, 3);
    }

    use super::*;#[test]
    fn test_is_jpeg_passing() {
        assert!(is_jpeg(&vec![0xFF, 0xD8]));
    }

    #[test]
    fn test_is_jpeg_failing() {
        assert!(!is_jpeg(&vec![0xFF, 0x00]));
    }

    #[test]
    fn number_of_used_bits_32() {
        let n = 0xFFFFFFFF / 2 + 1;
        assert_eq!(number_of_used_bits(&n), 32);
    }

    #[test]
    fn number_of_used_bits_4() {
        let n = 0xF;
        assert_eq!(number_of_used_bits(&n), 4);
    }

    #[test]
    fn number_of_used_bits_2() {
        let n = 3;
        assert_eq!(number_of_used_bits(&n), 2);
        assert_eq!(number_of_used_bits(&n), 2);
    }

    #[test]
    fn number_of_used_bits_1() {
        let n = 1;
        assert_eq!(number_of_used_bits(&n), 1);
    }

    #[test]
    fn number_of_used_bits_0() {
        let n = 0;
        assert_eq!(number_of_used_bits(&n), 0);
    }
}


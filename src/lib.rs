#[cfg(feature = "img")]
pub mod image;

use std::cmp::min;
use std::collections::HashMap;
use num::Signed;

const DEFAULT_CROP: f32 = 0.05;
const DEFAULT_GRID_SIZE: usize = 10;
const SIMILARITY_CUTOFF: f64 = 0.6;

/// Produces a 544 signed byte signature for a provided image, encoded as an array of conceptually
/// grouped RGBA bytes with the provided width. The result is designed to be compared to other
/// vectors computed by a call to this method using [cosine-similarity(a, b)].
pub fn get_buffer_signature(rgba_buffer: &[u8], width: usize) -> Vec<i8> {
    let gray = grayscale_buffer(rgba_buffer, width);
    compute_from_gray(gray, DEFAULT_CROP, DEFAULT_GRID_SIZE)
}

/// Produces a variable length signed byte signature for a provided image, encoded as an array of
/// conceptually grouped RGBA bytes with the provided width. The result is designed to be compared
/// to other vectors computed by a call to this method with identical tuning parameters using
/// [cosine-similarity(a, b)]. `crop` is a value in [0, 0.5] indicating what percentage of the image
/// to crop on all sides before grid placement. Note that this percentage is based not on the raw
/// width but a calculation of color density. `grid_size` indicates how many points to place on the
/// image for measurement in the resulting signature. Changing `grid_size` will alter the length of
/// the signature to `8 * (grid_size - 1)^2 - 12 * (grid_size - 3) - 20`.
pub fn get_tuned_buffer_signature(
    rgba_buffer: &[u8],
    width: usize,
    crop: f32,
    grid_size: usize,
) -> Vec<i8> {
    let gray = grayscale_buffer(rgba_buffer, width);
    compute_from_gray(gray, crop, grid_size)
}

/// Computes the cosine of the angle between two feature vectors. Those vectors must have been both
/// produced by calls to an un-tuned signature function or identical calls to a tuned version. Per
/// the source paper and out own research, when using the un-tuned signature calculation a cosine of
/// 0.6 or greater indicates significant similarity.
pub fn cosine_similarity(a: &Vec<i8>, b: &Vec<i8>) -> f64 {
    assert_eq!(a.len(), b.len());

    let dot_product: f64 = a.iter().zip(b.iter())
        .map(|(av, bv)| (av * bv) as f64)
        .sum();

    dot_product / (vector_length(a) * vector_length(b))
}

fn vector_length(v: &Vec<i8>) -> f64 {
    v.iter().map(|vi| (vi * vi) as f64).sum::<f64>().sqrt()
}

/// Core computation steps of image signatures. Descriptions for each step can be found on the
/// called functions.
fn compute_from_gray(gray: Vec<Vec<u8>>, crop: f32, grid_size: usize) -> Vec<i8> {
    let bounds = crop_boundaries(&gray, crop);
    let points = grid_points(&bounds, grid_size);
    let averages = grid_averages(gray, points, bounds);
    compute_signature(averages, grid_size)
}

/*
Step 1.
"If the image is color, we first convert it to 8-bit grayscale .. Pure white is represented by 255
and pure black by 0."
 */
fn grayscale_buffer(rgba_buffer: &[u8], width: usize) -> Vec<Vec<u8>> {
    let mut result = vec![];
    let mut idx: usize = 0;
    while idx < rgba_buffer.len() {
        let mut row = vec![];
        for _ in 0..width {
            let avg = pixel_gray(
                rgba_buffer[idx],
                rgba_buffer[idx + 1],
                rgba_buffer[idx + 2],
                rgba_buffer[idx + 3],
            );

            row.push(avg);
            idx += 4;
        }
        result.push(row);
    }

    result
}

fn pixel_gray(r: u8, g: u8, b: u8, a: u8) -> u8 {
    let rgb_avg = (r as u16 + g as u16 + b as u16) / 3;
    ((rgb_avg as f32) * (a as f32 / 255.0)) as u8
}

#[derive(Debug)]
struct Bounds {
    lower_x: usize,
    upper_x: usize,
    lower_y: usize,
    upper_y: usize,
}

/*
Step 2, part 1
"We define the grid in a way that is robust to mild
cropping, under the assumption that such cropping usually removes relatively featureless parts of
the image, for example, the margins of a document image or the dark bottom of the Mona Lisa picture.
For each column of the image, we compute the sum of absolute values of differences between adjacent
pixels in that column. We compute the total of all columns, and crop the image at the 5% and 95%
columns, that is, the columns such that 5% of the total sum of differences lies on either side of
the cropped image. We crop the rows of the image the same way (using the sums of original uncropped
rows).

For each column of the image, we compute the sum of absolute values of differences between
adjacent pixels in that column. We compute the total of all columns, and crop the image at
the 5% and 95% columns, that is, the columns such that 5% of the total sum of differences
lies on either side of the cropped image. We crop the rows of the image the same way"
(using the sums of original uncropped rows).
 */
fn crop_boundaries(pixels: &Vec<Vec<u8>>, crop: f32) -> Bounds {
    let row_diff_sums: Vec<i32> = (0..pixels.len()).map(|y|
        (1..pixels[y].len()).map(|x|
            pixels[y][x].abs_diff(pixels[y][x - 1]) as i32).sum()
    ).collect();

    let (top, bottom) = get_bounds(row_diff_sums, crop);

    let col_diff_sums: Vec<i32> = (0..pixels[0].len()).map(|x|
        (1..pixels.len()).map(|y|
            pixels[y][x].abs_diff(pixels[y - 1][x]) as i32).sum()
    ).collect();

    let (left, right) = get_bounds(col_diff_sums, crop);

    Bounds {
        lower_x: left,
        upper_x: right,
        lower_y: top,
        upper_y: bottom,
    }
}

fn get_bounds(diff_sums: Vec<i32>, crop: f32) -> (usize, usize) {
    let total_diff_sum: i32 = diff_sums.iter().map(|v| *v).sum();
    let threshold = (total_diff_sum as f32 * crop) as i32;
    let mut lower = 0;
    let mut upper = diff_sums.len() - 1;
    let mut sum = 0;

    while sum < threshold {
        sum += diff_sums[lower];
        lower += 1;
    }
    sum = 0;
    while sum < threshold {
        sum += diff_sums[upper];
        upper -= 1;
    }
    (lower, upper)
}

/*
Step 2, part 2
"We next impose a 9x9 grid of points on the image. (For large databases, a bigger grid such as 11x11
would give greater first-stage filtering.)
...
Conceptually, we then divide the cropped image into a 10x10 grid of blocks. We round each interior
grid point to the closest pixel (that is, integer coordinates), thereby setting a 􏰄 􏰗 􏰄 grid of
points on the image."
 */
fn grid_points(bounds: &Bounds, grid_size: usize) -> HashMap<(i8, i8), (usize, usize)> {
    let x_width = (bounds.upper_x - bounds.lower_x) / grid_size;
    let y_width = (bounds.upper_y - bounds.lower_y) / grid_size;

    let mut points = HashMap::new();
    for x in 1..grid_size {
        for y in 1..grid_size {
            points.insert((x as i8, y as i8), (x * x_width, y * y_width));
        }
    }

    points
}

/*
Step 3
"At each grid point, we compute the average gray level of the PxP square centered at the grid point.
We ran our experiments with P = max(2, floor(0.5 + min(n, m) / 20)) where n and m are the dimensions
of the image in pixels. The squares are slightly soft-edged, meaning that instead of using the
pixel’s gray levels themselves, we use an average of a 􏰏 􏰗 􏰏 block centered at that pixel."
 */
fn grid_averages(
    pixels: Vec<Vec<u8>>,
    points: HashMap<(i8, i8), (usize, usize)>,
    bounds: Bounds,
) -> HashMap<(i8, i8), u8> {
    let x_width = bounds.upper_x - bounds.lower_x;
    let y_width = bounds.upper_y - bounds.lower_y;
    let square_edge = (max(
        2.0,
        (0.5 + min(x_width, y_width) as f32 / 20.0).floor(),
    ) / 2.0) as i32;

    let mut result = HashMap::new();
    for (grid_coord, (point_x, point_y)) in points {
        let mut sum: f32 = 0.0;
        for delta_x in -square_edge..=square_edge {
            for delta_y in -square_edge..=square_edge {
                let average = pixel_average(
                    &pixels,
                    (point_x as i32 + delta_x) as usize,
                    (point_y as i32 + delta_y) as usize,
                );
                sum += average;
            }
        }

        let i = sum / ((square_edge * 2 + 1) * (square_edge * 2 + 1)) as f32;
        result.insert(grid_coord, i as u8);
    }

    result
}

//Sins, crimes, etc
fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/*
Step 4
For each grid point, we compute an 8-element array whose elements give a comparison of the average
gray level of the grid point square with those of its eight neighbors. The result of a comparison
can be “much darker”, “darker”, “same”, “lighter”, or “much lighter”, represented numerically as
-2, -1, 0, 1 and 2, respectively. The “same” values are those averages that differ by no more than
2 on a scale of 0 to 255. We set the boundary between “much darker” and “darker” so that these two
values are equally popular; we do the same for “lighter” and “much lighter”. The rationale in this
step is that “same” may be very common in images with flat backgrounds (such as text documents), and
hence it should not be included in the histogram equalization applied to the other values. Grid
points in the first or last rows or column have fewer than 8 neighbors..."

(The authors pad missing neighbors with 0's, we just omit them.)

Step 5
"The signature of an image is simply the concatenation of the 8-element arrays corresponding to the
grid points, ordered left-to-right, top-to-bottom..."
*/
fn compute_signature(point_averages: HashMap<(i8, i8), u8>, grid_size: usize) -> Vec<i8> {
    let mut raw_diffs = vec![];
    for grid_y in 1..(grid_size as i8) {
        for grid_x in 1..(grid_size as i8) {
            let gray = *point_averages.get(&(grid_x, grid_y)).unwrap();
            let raw_point_diffs: Vec<i16> = [
                (-1, -1), (0, -1), (1, -1),
                (-1, 0), (1, 0),
                (-1, 1), (0, 1), (1, 1)
            ].iter().filter_map(|(delta_x, delta_y)| {
                if let Some(other) = point_averages.get(&(grid_x + delta_x, grid_y + delta_y)) {
                    Some(compute_diff(gray, *other))
                } else {
                    None
                }
            }).collect();
            raw_diffs.push(raw_point_diffs)
        }
    }

    let (dark_threshold, light_threshold) = get_thresholds(&raw_diffs);
    raw_diffs.into_iter().map(|neighbors|
        neighbors.into_iter()
            .map(|v| {
                if v > 0 {
                    collapse(v, light_threshold)
                } else if v < 0 {
                    collapse(v, dark_threshold)
                } else {
                    0
                }
            })
    ).flatten().collect()
}


fn get_thresholds(raw_diffs: &Vec<Vec<i16>>) -> (i16, i16) {
    let (dark, light): (Vec<i16>, Vec<i16>) = raw_diffs.iter().flatten()
        .filter(|d| **d != 0)
        .partition(|d| **d < 0);

    let dark_threshold = get_median(dark);
    let light_threshold = get_median(light);

    (dark_threshold, light_threshold)
}

fn collapse(val: i16, threshold: i16) -> i8 {
    if val.abs() >= threshold.abs() {
        2 * val.signum() as i8
    } else {
        val.signum() as i8
    }
}

fn get_median(mut vec: Vec<i16>) -> i16 {
    vec.sort();
    if vec.len() % 2 == 0 {
        if vec.is_empty() {
            0
        } else {
            (vec[(vec.len() / 2) - 1] + vec[vec.len() / 2]) / 2
        }
    } else {
        vec[vec.len() / 2]
    }
}

fn compute_diff(me: u8, other: u8) -> i16 {
    let raw_result = me as i16 - other as i16;
    if raw_result.abs() <= 2 {
        0
    } else {
        raw_result
    }
}

fn pixel_average(pixels: &Vec<Vec<u8>>, x: usize, y: usize) -> f32 {
    let sum: f32 = [
        (-1, -1), (0, -1), (1, -1),
        (-1, 0), (0, 0), (1, 0),
        (-1, 1), (0, 1), (1, 1)
    ].iter().map(|(delta_x, delta_y)| {
        pixels[(y as i32 + delta_y) as usize][(x as i32 + delta_x) as usize] as f32
    }).sum();

    sum / 9.0
}

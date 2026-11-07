//! Renderer spike: a 5,000-row `ListView`, each row a 96 px image and two text lines.
//! How to run and what to record: `docs/measurements.md`.

use slint::{Image, Model, ModelRc, ModelTracker, Rgba8Pixel, SharedPixelBuffer};

slint::slint! {
    import { ListView } from "std-widgets.slint";

    export struct Row { art: image, title: string, subtitle: string }

    export component Spike inherits Window {
        in property <[Row]> rows;
        in property <bool> autoscroll;
        title: "Hush renderer spike";
        preferred-width: 1280px;
        preferred-height: 800px;
        background: #1c1c1e;

        list := ListView {
            for row in rows: HorizontalLayout {
                height: 112px;
                padding: 8px;
                spacing: 12px;
                Image { source: row.art; width: 96px; height: 96px; }
                VerticalLayout {
                    alignment: center;
                    Text { text: row.title; color: #ffffff; font-size: 14px; font-weight: 600; }
                    Text { text: row.subtitle; color: #ffffff99; font-size: 13px; }
                }
            }
        }

        // ~3,400 px/s at 144 Hz: a repeatable stand-in for a flick-scroll.
        Timer {
            interval: 7ms;
            running: autoscroll;
            triggered => {
                list.content-y = list.content-height - list.visible-height + list.content-y < 24px
                    ? 0px
                    : list.content-y - 24px;
            }
        }
    }
}

const ROWS: usize = 5_000;
const ART: u32 = 96;
const PALETTE: u8 = 64;

/// Rows built on demand, like the real list model: only visible rows exist as strings.
struct Rows {
    art: Vec<Image>,
}

impl Model for Rows {
    type Data = Row;

    fn row_count(&self) -> usize {
        ROWS
    }

    fn row_data(&self, i: usize) -> Option<Row> {
        (i < ROWS).then(|| Row {
            art: self.art[i % self.art.len()].clone(),
            title: format!("Track {}", i + 1).into(),
            subtitle: format!("Artist {} — Album {}", i % 97, i % 389).into(),
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &()
    }
}

fn art(seed: u8) -> Image {
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(ART, ART);
    let (lines, _) = buf.make_mut_slice().as_chunks_mut::<{ ART as usize }>();
    for (y, line) in (0u8..).zip(lines) {
        for (x, px) in (0u8..).zip(line) {
            *px = Rgba8Pixel {
                r: x.wrapping_mul(2).wrapping_add(seed.wrapping_mul(4)),
                g: y.wrapping_mul(2),
                b: seed.wrapping_mul(4),
                a: 255,
            };
        }
    }
    Image::from_rgba8(buf)
}

fn main() -> Result<(), slint::PlatformError> {
    let spike = Spike::new()?;
    spike.set_rows(ModelRc::new(Rows {
        art: (0..PALETTE).map(art).collect(),
    }));
    spike.set_autoscroll(std::env::args().any(|a| a == "--autoscroll"));
    spike.run()
}

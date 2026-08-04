use crate::bgra_image::PixelRectangle;

#[derive(Default)]
pub struct FastMonoCleanup {
    pending: Option<PixelRectangle>,
}

impl FastMonoCleanup {
    pub fn include_update(&mut self, rectangle: PixelRectangle) {
        self.pending = Some(
            self.pending
                .map_or(rectangle, |pending| pending.include(rectangle)),
        );
    }

    pub fn take_color_update(&mut self, changed: Option<PixelRectangle>) -> Option<PixelRectangle> {
        match (self.pending.take(), changed) {
            (Some(pending), Some(changed)) => Some(pending.include(changed)),
            (Some(pending), None) => Some(pending),
            (None, changed) => changed,
        }
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_content_needs_no_color_update() {
        assert_eq!(FastMonoCleanup::default().take_color_update(None), None);
    }

    #[test]
    fn color_update_cleans_every_fast_mono_rectangle() {
        let mut cleanup = FastMonoCleanup::default();
        cleanup.include_update(PixelRectangle {
            x: 10,
            y: 20,
            width: 5,
            height: 10,
        });
        cleanup.include_update(PixelRectangle {
            x: 30,
            y: 5,
            width: 10,
            height: 5,
        });

        assert_eq!(
            cleanup.take_color_update(None),
            Some(PixelRectangle {
                x: 10,
                y: 5,
                width: 30,
                height: 25,
            })
        );
        assert_eq!(cleanup.take_color_update(None), None);
    }

    #[test]
    fn changed_pixels_and_fast_mono_cleanup_share_one_update() {
        let mut cleanup = FastMonoCleanup::default();
        cleanup.include_update(PixelRectangle {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        });
        assert_eq!(
            cleanup.take_color_update(Some(PixelRectangle {
                x: 20,
                y: 20,
                width: 5,
                height: 5,
            })),
            Some(PixelRectangle {
                x: 0,
                y: 0,
                width: 25,
                height: 25,
            })
        );
    }
}

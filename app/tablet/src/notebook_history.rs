impl Notebook {
    fn current_page_history_availability(&self) -> remarque_page_log::HistoryAvailability {
        if let Some(session) = &self.shared_page
            && self.current_page_identity().as_ref() == Some(&session.identity)
        {
            return session.history;
        }
        self.current_page_identity().map_or_else(
            remarque_page_log::HistoryAvailability::default,
            |identity| self.page_edit_histories.availability(&identity),
        )
    }

    fn record_current_page_edit(&mut self, before: &[Stroke]) -> io::Result<()> {
        if self.current_page_is_shared() {
            return Ok(());
        }
        let identity = self
            .current_page_identity()
            .ok_or_else(|| io::Error::other("no current page identity"))?;
        let after = self.page()?.strokes.clone();
        self.page_edit_histories.record(identity, before, &after);
        self.redraw_toolbar()
    }

    fn undo_current_page(&mut self) -> io::Result<()> {
        if !self.current_page_history_availability().can_undo {
            return Ok(());
        }
        if self.current_page_is_shared() {
            return self.submit_shared_operation_without_visible_pixels(
                SubmittedPageOperation::Undo,
            );
        }
        let identity = self
            .current_page_identity()
            .ok_or_else(|| io::Error::other("no current page identity"))?;
        let mut strokes = self.page()?.strokes.clone();
        if self.page_edit_histories.undo(&identity, &mut strokes)? {
            self.replace_current_page_strokes_after_history(strokes)?;
        }
        Ok(())
    }

    fn redo_current_page(&mut self) -> io::Result<()> {
        if !self.current_page_history_availability().can_redo {
            return Ok(());
        }
        if self.current_page_is_shared() {
            return self.submit_shared_operation_without_visible_pixels(
                SubmittedPageOperation::Redo,
            );
        }
        let identity = self
            .current_page_identity()
            .ok_or_else(|| io::Error::other("no current page identity"))?;
        let mut strokes = self.page()?.strokes.clone();
        if self.page_edit_histories.redo(&identity, &mut strokes)? {
            self.replace_current_page_strokes_after_history(strokes)?;
        }
        Ok(())
    }

    fn replace_current_page_strokes_after_history(
        &mut self,
        strokes: Vec<Stroke>,
    ) -> io::Result<()> {
        self.page_mut()?.strokes = strokes;
        self.save_state()?;
        let changed = self.redraw_notebook()?;
        self.display.submit_mode_four_color(changed);
        Ok(())
    }
}

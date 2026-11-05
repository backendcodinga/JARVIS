//! Deciding that the phrase was said, from one score per 80 ms hop. Pure logic: no model needed.

/// What the spike measured as a good start (D-29): a score of 0.7 or more twice in a row.
pub const DEFAULT_THRESHOLD: f32 = 0.7;
pub const DEFAULT_NEEDED: u32 = 2;
/// After a trigger, ignore the detector for this many hops (25 hops are 2 s), so one phrase is one
/// trigger and the assistant's own voice or the same burst cannot fire it twice.
pub const DEFAULT_COOLDOWN_HOPS: u32 = 25;

#[derive(Debug, Clone)]
pub struct Trigger {
    threshold: f32,
    needed: u32,
    cooldown_hops: u32,
    run: u32,
    cooling: u32,
}

impl Default for Trigger {
    fn default() -> Self {
        Self::new(DEFAULT_THRESHOLD, DEFAULT_NEEDED, DEFAULT_COOLDOWN_HOPS)
    }
}

impl Trigger {
    pub fn new(threshold: f32, needed: u32, cooldown_hops: u32) -> Self {
        Self {
            threshold,
            needed: needed.max(1),
            cooldown_hops,
            run: 0,
            cooling: 0,
        }
    }

    /// Feeds one score; `true` means "the phrase was just said".
    pub fn push(&mut self, score: f32) -> bool {
        if self.cooling > 0 {
            self.cooling -= 1;
            self.run = 0;
            return false;
        }
        // A score that is not a number (a broken model output) never counts.
        if score >= self.threshold {
            self.run += 1;
            if self.run >= self.needed {
                self.run = 0;
                self.cooling = self.cooldown_hops;
                return true;
            }
        } else {
            self.run = 0;
        }
        false
    }

    /// Back to the start (a new listening session, or the microphone was muted).
    pub fn reset(&mut self) {
        self.run = 0;
        self.cooling = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fires(t: &mut Trigger, scores: &[f32]) -> Vec<usize> {
        scores
            .iter()
            .enumerate()
            .filter_map(|(i, s)| t.push(*s).then_some(i))
            .collect()
    }

    #[test]
    fn it_needs_the_threshold_twice_in_a_row() {
        let mut t = Trigger::default();
        // One high score is a blip; two in a row is the phrase.
        assert_eq!(
            fires(&mut t, &[0.0, 0.9, 0.1, 0.9, 0.1]),
            Vec::<usize>::new()
        );
        assert_eq!(fires(&mut t, &[0.0, 0.75, 0.8]), vec![2]);
    }

    #[test]
    fn the_threshold_itself_counts_and_just_under_does_not() {
        let mut t = Trigger::default();
        assert_eq!(fires(&mut t, &[0.7, 0.7]), vec![1]);
        let mut t = Trigger::default();
        assert!(fires(&mut t, &[0.699, 0.699, 0.699]).is_empty());
    }

    #[test]
    fn one_phrase_is_one_trigger_and_the_cooldown_ends() {
        let mut t = Trigger::new(0.7, 2, 4);
        // A long burst fires once; the next four hops are ignored; then it can fire again.
        let burst = [0.9; 12];
        let at = fires(&mut t, &burst);
        // Fires at hop 1, ignores hops 2 to 5, then needs two more in a row: hops 6 and 7.
        assert_eq!(at, vec![1, 7]);
    }

    #[test]
    fn a_score_that_is_not_a_number_never_counts() {
        let mut t = Trigger::default();
        assert!(fires(&mut t, &[f32::NAN, f32::NAN, f32::NAN]).is_empty());
        // And it breaks a run: 0.9, NaN, 0.9 is not two in a row.
        assert!(fires(&mut t, &[0.9, f32::NAN, 0.9]).is_empty());
        assert!(fires(&mut t, &[f32::INFINITY, f32::INFINITY]).len() == 1);
    }

    #[test]
    fn reset_forgets_a_half_run_and_a_cooldown() {
        let mut t = Trigger::default();
        assert!(!t.push(0.9));
        t.reset();
        assert!(!t.push(0.9), "the run before the reset is gone");
        assert!(t.push(0.9));
        t.reset();
        assert!(
            !t.push(0.9) && t.push(0.9),
            "a reset also ends the cooldown"
        );
    }

    #[test]
    fn asking_for_zero_in_a_row_still_needs_one() {
        let mut t = Trigger::new(0.7, 0, 0);
        assert!(!t.push(0.1));
        assert!(t.push(0.9));
    }
}

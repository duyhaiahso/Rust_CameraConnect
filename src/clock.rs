use std::time::{SystemTime, UNIX_EPOCH};

/// Vietnam business time is fixed UTC+7; it does not use the workstation's
/// possibly incorrect local timezone setting.
pub struct Timestamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
}

impl Timestamp {
    pub fn now() -> Self {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let seconds = elapsed.as_secs() as i64 + 7 * 3600;
        let days = seconds.div_euclid(86_400);
        let day_seconds = seconds.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: day_seconds / 3600,
            minute: (day_seconds % 3600) / 60,
            second: day_seconds % 60,
            millisecond: elapsed.subsec_millis(),
        }
    }

    pub fn display(&self) -> String {
        format!(
            "{:02}/{:02}/{:04} {:02}:{:02}:{:02}",
            self.day, self.month, self.year, self.hour, self.minute, self.second
        )
    }

    pub fn iso8601(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}+07:00",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millisecond
        )
    }

    pub fn date_key(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn filename_key(&self) -> String {
        format!(
            "{:04}{:02}{:02}_{:02}{:02}{:02}_{:03}",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millisecond
        )
    }
}

// Gregorian calendar conversion, where day 0 is 1970-01-01.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn calendar_boundaries() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_453), (2025, 12, 31));
        assert_eq!(civil_from_days(20_454), (2026, 1, 1));
    }
}

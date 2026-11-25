//! Month arithmetic for the calendar tab (pure, no date crate).

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 31,
    }
}

/// 0 = Sunday (Sakamoto's algorithm).
pub fn weekday_of_first(y: i32, m: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let m = m.clamp(1, 12);
    let y = if m < 3 { y - 1 } else { y };
    ((y + y / 4 - y / 100 + y / 400 + T[m as usize - 1] + 1).rem_euclid(7)) as u32
}

/// 6 weeks of 7 days, Sunday first; `None` outside the month.
pub fn month_grid(y: i32, m: u32) -> [[Option<u8>; 7]; 6] {
    let (first, days) = (weekday_of_first(y, m) as usize, days_in_month(y, m) as usize);
    let mut grid = [[None; 7]; 6];
    for d in 0..days {
        let cell = first + d;
        grid[cell / 7][cell % 7] = Some(d as u8 + 1);
    }
    grid
}

/// Moves `delta` months from `month` (year, 1..=12).
pub fn shift(month: (i32, u32), delta: i32) -> (i32, u32) {
    let index = month.0 * 12 + month.1 as i32 - 1 + delta;
    (index.div_euclid(12), index.rem_euclid(12) as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_lengths() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2026, 11), 30);
        assert_eq!(days_in_month(2026, 12), 31);
    }

    #[test]
    fn first_weekday() {
        assert_eq!(weekday_of_first(2026, 11), 0);
        assert_eq!(weekday_of_first(2026, 12), 2);
    }

    #[test]
    fn grid_layout() {
        let g = month_grid(2026, 11);
        assert_eq!(g[0][0], Some(1));
        assert_eq!(g[4][1], Some(30));
        assert!(g[4][2..].iter().all(|d| d.is_none()));
        assert!(g[5].iter().all(|d| d.is_none()));
        // A 31-day month starting on Saturday fills the sixth row.
        let g = month_grid(2026, 8);
        assert_eq!(g[0][6], Some(1));
        assert_eq!(g[5][1], Some(31));
    }

    #[test]
    fn shifts_across_years() {
        assert_eq!(shift((2026, 12), 1), (2027, 1));
        assert_eq!(shift((2026, 1), -1), (2025, 12));
        assert_eq!(shift((2026, 6), 0), (2026, 6));
        assert_eq!(shift((2026, 6), 13), (2027, 7));
    }
}

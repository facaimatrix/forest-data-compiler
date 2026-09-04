//! Offline country suggestions from plot coordinates.
//!
//! Uses axis-aligned bounding boxes and, when several boxes contain a point,
//! the smallest box. This is a suggestion, not a cadastral lookup — review
//! plots near borders.

#[derive(Clone, Copy)]
struct CountryBox {
    name: &'static str,
    continent: &'static str,
    min_lat: f64,
    max_lat: f64,
    min_lon: f64,
    max_lon: f64,
}

impl CountryBox {
    fn contains(self, lat: f64, lon: f64) -> bool {
        lat >= self.min_lat && lat <= self.max_lat && lon >= self.min_lon && lon <= self.max_lon
    }

    fn area(self) -> f64 {
        (self.max_lat - self.min_lat) * (self.max_lon - self.min_lon)
    }
}

/// Compact boxes for countries that commonly host forest inventory plots.
/// Overseas territories are listed separately from the metropolitan state.
const BOXES: &[CountryBox] = &[
    CountryBox { name: "Argentina", continent: "South America", min_lat: -55.1, max_lat: -21.8, min_lon: -73.6, max_lon: -53.6 },
    CountryBox { name: "Australia", continent: "Oceania", min_lat: -43.7, max_lat: -10.6, min_lon: 113.1, max_lon: 153.7 },
    CountryBox { name: "Austria", continent: "Europe", min_lat: 46.4, max_lat: 49.0, min_lon: 9.5, max_lon: 17.2 },
    CountryBox { name: "Bangladesh", continent: "Asia", min_lat: 20.7, max_lat: 26.6, min_lon: 88.0, max_lon: 92.7 },
    CountryBox { name: "Belgium", continent: "Europe", min_lat: 49.5, max_lat: 51.5, min_lon: 2.5, max_lon: 6.4 },
    CountryBox { name: "Belize", continent: "North America", min_lat: 15.9, max_lat: 18.5, min_lon: -89.3, max_lon: -87.5 },
    CountryBox { name: "Benin", continent: "Africa", min_lat: 6.2, max_lat: 12.4, min_lon: 0.8, max_lon: 3.9 },
    CountryBox { name: "Bolivia", continent: "South America", min_lat: -22.9, max_lat: -9.7, min_lon: -69.6, max_lon: -57.5 },
    CountryBox { name: "Botswana", continent: "Africa", min_lat: -26.9, max_lat: -17.8, min_lon: 20.0, max_lon: 29.4 },
    CountryBox { name: "Brazil", continent: "South America", min_lat: -33.8, max_lat: 5.3, min_lon: -73.9, max_lon: -34.8 },
    CountryBox { name: "Cambodia", continent: "Asia", min_lat: 10.4, max_lat: 14.7, min_lon: 102.3, max_lon: 107.6 },
    CountryBox { name: "Cameroon", continent: "Africa", min_lat: 1.7, max_lat: 13.1, min_lon: 8.5, max_lon: 16.2 },
    CountryBox { name: "Canada", continent: "North America", min_lat: 41.7, max_lat: 83.1, min_lon: -141.0, max_lon: -52.6 },
    CountryBox { name: "Central African Republic", continent: "Africa", min_lat: 2.2, max_lat: 11.0, min_lon: 14.4, max_lon: 27.5 },
    CountryBox { name: "Chile", continent: "South America", min_lat: -55.9, max_lat: -17.5, min_lon: -75.6, max_lon: -66.4 },
    CountryBox { name: "China", continent: "Asia", min_lat: 18.2, max_lat: 53.6, min_lon: 73.5, max_lon: 134.8 },
    CountryBox { name: "Colombia", continent: "South America", min_lat: -4.3, max_lat: 13.4, min_lon: -79.0, max_lon: -66.9 },
    CountryBox { name: "Congo", continent: "Africa", min_lat: -5.1, max_lat: 3.7, min_lon: 11.1, max_lon: 18.7 },
    CountryBox { name: "Costa Rica", continent: "North America", min_lat: 8.0, max_lat: 11.2, min_lon: -85.9, max_lon: -82.6 },
    CountryBox { name: "Cuba", continent: "North America", min_lat: 19.8, max_lat: 23.3, min_lon: -85.0, max_lon: -74.1 },
    CountryBox { name: "Czechia", continent: "Europe", min_lat: 48.6, max_lat: 51.1, min_lon: 12.1, max_lon: 18.9 },
    CountryBox { name: "Democratic Republic of the Congo", continent: "Africa", min_lat: -13.5, max_lat: 5.4, min_lon: 12.2, max_lon: 31.3 },
    CountryBox { name: "Denmark", continent: "Europe", min_lat: 54.6, max_lat: 57.8, min_lon: 8.1, max_lon: 15.2 },
    CountryBox { name: "Ecuador", continent: "South America", min_lat: -5.0, max_lat: 1.7, min_lon: -81.1, max_lon: -75.2 },
    CountryBox { name: "Estonia", continent: "Europe", min_lat: 57.5, max_lat: 59.7, min_lon: 21.8, max_lon: 28.2 },
    CountryBox { name: "Ethiopia", continent: "Africa", min_lat: 3.4, max_lat: 14.9, min_lon: 33.0, max_lon: 48.0 },
    CountryBox { name: "Finland", continent: "Europe", min_lat: 59.8, max_lat: 70.1, min_lon: 20.6, max_lon: 31.6 },
    CountryBox { name: "France", continent: "Europe", min_lat: 42.3, max_lat: 51.1, min_lon: -5.1, max_lon: 8.2 },
    CountryBox { name: "French Guiana", continent: "South America", min_lat: 2.1, max_lat: 5.8, min_lon: -54.6, max_lon: -51.6 },
    CountryBox { name: "Gabon", continent: "Africa", min_lat: -4.0, max_lat: 2.3, min_lon: 8.7, max_lon: 14.5 },
    CountryBox { name: "Germany", continent: "Europe", min_lat: 47.3, max_lat: 55.1, min_lon: 5.9, max_lon: 15.0 },
    CountryBox { name: "Ghana", continent: "Africa", min_lat: 4.7, max_lat: 11.2, min_lon: -3.3, max_lon: 1.2 },
    CountryBox { name: "Greece", continent: "Europe", min_lat: 34.8, max_lat: 41.8, min_lon: 19.4, max_lon: 28.2 },
    CountryBox { name: "Guatemala", continent: "North America", min_lat: 13.7, max_lat: 17.8, min_lon: -92.3, max_lon: -88.2 },
    CountryBox { name: "Guyana", continent: "South America", min_lat: 1.2, max_lat: 8.6, min_lon: -61.4, max_lon: -56.5 },
    CountryBox { name: "Honduras", continent: "North America", min_lat: 13.0, max_lat: 16.5, min_lon: -89.4, max_lon: -83.2 },
    CountryBox { name: "Hungary", continent: "Europe", min_lat: 45.7, max_lat: 48.6, min_lon: 16.1, max_lon: 22.9 },
    CountryBox { name: "India", continent: "Asia", min_lat: 6.7, max_lat: 35.5, min_lon: 68.1, max_lon: 97.4 },
    CountryBox { name: "Indonesia", continent: "Asia", min_lat: -11.0, max_lat: 6.1, min_lon: 95.0, max_lon: 141.0 },
    CountryBox { name: "Ireland", continent: "Europe", min_lat: 51.4, max_lat: 55.4, min_lon: -10.5, max_lon: -6.0 },
    CountryBox { name: "Italy", continent: "Europe", min_lat: 36.6, max_lat: 47.1, min_lon: 6.6, max_lon: 18.5 },
    CountryBox { name: "Ivory Coast", continent: "Africa", min_lat: 4.4, max_lat: 10.7, min_lon: -8.6, max_lon: -2.5 },
    CountryBox { name: "Japan", continent: "Asia", min_lat: 24.0, max_lat: 45.5, min_lon: 122.9, max_lon: 145.8 },
    CountryBox { name: "Kenya", continent: "Africa", min_lat: -4.7, max_lat: 5.0, min_lon: 33.9, max_lon: 41.9 },
    CountryBox { name: "Laos", continent: "Asia", min_lat: 13.9, max_lat: 22.5, min_lon: 100.1, max_lon: 107.7 },
    CountryBox { name: "Latvia", continent: "Europe", min_lat: 55.7, max_lat: 58.1, min_lon: 20.9, max_lon: 28.2 },
    CountryBox { name: "Liberia", continent: "Africa", min_lat: 4.3, max_lat: 8.6, min_lon: -11.5, max_lon: -7.4 },
    CountryBox { name: "Lithuania", continent: "Europe", min_lat: 53.9, max_lat: 56.4, min_lon: 20.9, max_lon: 26.8 },
    CountryBox { name: "Madagascar", continent: "Africa", min_lat: -25.6, max_lat: -11.9, min_lon: 43.2, max_lon: 50.5 },
    CountryBox { name: "Malaysia", continent: "Asia", min_lat: 0.9, max_lat: 7.4, min_lon: 99.6, max_lon: 119.3 },
    CountryBox { name: "Mexico", continent: "North America", min_lat: 14.5, max_lat: 32.7, min_lon: -118.4, max_lon: -86.7 },
    CountryBox { name: "Mozambique", continent: "Africa", min_lat: -26.9, max_lat: -10.5, min_lon: 30.2, max_lon: 40.8 },
    CountryBox { name: "Myanmar", continent: "Asia", min_lat: 9.6, max_lat: 28.5, min_lon: 92.2, max_lon: 101.2 },
    CountryBox { name: "Namibia", continent: "Africa", min_lat: -29.0, max_lat: -16.9, min_lon: 11.7, max_lon: 25.3 },
    CountryBox { name: "Nepal", continent: "Asia", min_lat: 26.3, max_lat: 30.4, min_lon: 80.0, max_lon: 88.2 },
    CountryBox { name: "Netherlands", continent: "Europe", min_lat: 50.8, max_lat: 53.5, min_lon: 3.4, max_lon: 7.2 },
    CountryBox { name: "New Zealand", continent: "Oceania", min_lat: -47.3, max_lat: -34.4, min_lon: 166.4, max_lon: 178.6 },
    CountryBox { name: "Nicaragua", continent: "North America", min_lat: 10.7, max_lat: 15.0, min_lon: -87.7, max_lon: -82.6 },
    CountryBox { name: "Nigeria", continent: "Africa", min_lat: 4.3, max_lat: 13.9, min_lon: 2.7, max_lon: 14.7 },
    CountryBox { name: "Norway", continent: "Europe", min_lat: 58.0, max_lat: 71.2, min_lon: 4.6, max_lon: 31.1 },
    CountryBox { name: "Panama", continent: "North America", min_lat: 7.2, max_lat: 9.6, min_lon: -83.0, max_lon: -77.2 },
    CountryBox { name: "Papua New Guinea", continent: "Oceania", min_lat: -11.7, max_lat: -1.3, min_lon: 140.8, max_lon: 155.6 },
    CountryBox { name: "Paraguay", continent: "South America", min_lat: -27.6, max_lat: -19.3, min_lon: -62.6, max_lon: -54.3 },
    CountryBox { name: "Peru", continent: "South America", min_lat: -18.4, max_lat: -0.0, min_lon: -81.3, max_lon: -68.7 },
    CountryBox { name: "Philippines", continent: "Asia", min_lat: 4.6, max_lat: 21.1, min_lon: 116.9, max_lon: 126.6 },
    CountryBox { name: "Poland", continent: "Europe", min_lat: 49.0, max_lat: 54.8, min_lon: 14.1, max_lon: 24.1 },
    CountryBox { name: "Portugal", continent: "Europe", min_lat: 36.96, max_lat: 42.15, min_lon: -9.53, max_lon: -6.19 },
    CountryBox { name: "Romania", continent: "Europe", min_lat: 43.6, max_lat: 48.3, min_lon: 20.3, max_lon: 29.7 },
    CountryBox { name: "Russia", continent: "Europe", min_lat: 41.2, max_lat: 81.9, min_lon: 27.3, max_lon: 180.0 },
    CountryBox { name: "Rwanda", continent: "Africa", min_lat: -2.8, max_lat: -1.0, min_lon: 28.9, max_lon: 30.9 },
    CountryBox { name: "Senegal", continent: "Africa", min_lat: 12.3, max_lat: 16.7, min_lon: -17.5, max_lon: -11.4 },
    CountryBox { name: "Slovakia", continent: "Europe", min_lat: 47.7, max_lat: 49.6, min_lon: 16.8, max_lon: 22.6 },
    CountryBox { name: "Slovenia", continent: "Europe", min_lat: 45.4, max_lat: 46.9, min_lon: 13.4, max_lon: 16.6 },
    CountryBox { name: "South Africa", continent: "Africa", min_lat: -34.8, max_lat: -22.1, min_lon: 16.5, max_lon: 32.9 },
    CountryBox { name: "South Korea", continent: "Asia", min_lat: 33.1, max_lat: 38.6, min_lon: 125.9, max_lon: 129.6 },
    CountryBox { name: "Spain", continent: "Europe", min_lat: 36.0, max_lat: 43.8, min_lon: -9.3, max_lon: 3.3 },
    CountryBox { name: "Sri Lanka", continent: "Asia", min_lat: 5.9, max_lat: 9.8, min_lon: 79.7, max_lon: 81.9 },
    CountryBox { name: "Suriname", continent: "South America", min_lat: 1.8, max_lat: 6.0, min_lon: -58.1, max_lon: -54.0 },
    CountryBox { name: "Sweden", continent: "Europe", min_lat: 55.3, max_lat: 69.1, min_lon: 11.1, max_lon: 24.2 },
    CountryBox { name: "Switzerland", continent: "Europe", min_lat: 45.8, max_lat: 47.8, min_lon: 5.96, max_lon: 10.5 },
    CountryBox { name: "Tanzania", continent: "Africa", min_lat: -11.8, max_lat: -1.0, min_lon: 29.3, max_lon: 40.4 },
    CountryBox { name: "Thailand", continent: "Asia", min_lat: 5.6, max_lat: 20.5, min_lon: 97.3, max_lon: 105.6 },
    CountryBox { name: "Uganda", continent: "Africa", min_lat: -1.5, max_lat: 4.2, min_lon: 29.6, max_lon: 35.0 },
    CountryBox { name: "United Kingdom", continent: "Europe", min_lat: 49.9, max_lat: 58.7, min_lon: -8.2, max_lon: 1.8 },
    CountryBox { name: "United States", continent: "North America", min_lat: 24.5, max_lat: 49.4, min_lon: -125.0, max_lon: -66.9 },
    CountryBox { name: "Alaska", continent: "North America", min_lat: 51.2, max_lat: 71.4, min_lon: -179.1, max_lon: -129.9 },
    CountryBox { name: "Uruguay", continent: "South America", min_lat: -35.0, max_lat: -30.1, min_lon: -58.5, max_lon: -53.1 },
    CountryBox { name: "Venezuela", continent: "South America", min_lat: 0.6, max_lat: 12.2, min_lon: -73.4, max_lon: -59.8 },
    CountryBox { name: "Vietnam", continent: "Asia", min_lat: 8.6, max_lat: 23.4, min_lon: 102.1, max_lon: 109.5 },
    CountryBox { name: "Zambia", continent: "Africa", min_lat: -18.1, max_lat: -8.2, min_lon: 21.9, max_lon: 33.7 },
    CountryBox { name: "Zimbabwe", continent: "Africa", min_lat: -22.4, max_lat: -15.6, min_lon: 25.2, max_lon: 33.1 },
];

pub fn country_at(lat: f64, lon: f64) -> Option<(&'static str, &'static str)> {
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    let mut best: Option<CountryBox> = None;
    for box_ in BOXES {
        if !box_.contains(lat, lon) {
            continue;
        }
        best = Some(match best {
            None => *box_,
            Some(current) if box_.area() < current.area() => *box_,
            Some(current) => current,
        });
    }
    best.map(|b| {
        let name = if b.name == "Alaska" { "United States" } else { b.name };
        (name, b.continent)
    })
}

/// Distinct countries (and their continents) for a set of plot coordinates.
pub fn suggest_from_coordinates(points: &[(f64, f64)]) -> (Vec<String>, Vec<String>) {
    let mut countries = std::collections::BTreeMap::<String, u32>::new();
    let mut continents = std::collections::BTreeSet::<String>::new();
    for (lat, lon) in points {
        if let Some((country, continent)) = country_at(*lat, *lon) {
            *countries.entry(country.to_string()).or_insert(0) += 1;
            continents.insert(continent.to_string());
        }
    }
    (
        countries.into_keys().collect(),
        continents.into_iter().collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amazon_plot_is_brazil() {
        let (name, continent) = country_at(-9.12879, -55.18665).unwrap();
        assert_eq!(name, "Brazil");
        assert_eq!(continent, "South America");
    }

    #[test]
    fn paris_is_france() {
        assert_eq!(country_at(48.8566, 2.3522).unwrap().0, "France");
    }
}

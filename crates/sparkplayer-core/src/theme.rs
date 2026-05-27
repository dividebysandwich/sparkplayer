use std::cell::Cell;

use ratatui::style::Color;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub name: &'static str,
    pub label: &'static str,
    /// Primary highlight color — used for focused borders, "Spark" branding,
    /// progress gauge, and the most prominent accent across the UI.
    pub primary: Color,
    /// Secondary accent — used for prefixes, focused titles, browser
    /// directory entries, and the volume bar legend.
    pub accent: Color,
    /// Used for unfocused borders, metadata labels, footer borders.
    pub secondary: Color,
    /// Used for the now-playing track row, album-art title, repeat badge,
    /// and the volume percentage readout.
    pub highlight: Color,
    /// Status/OK indicator — drives the "Playing" badge and the bottom of
    /// the FFT bar gradient.
    pub ok: Color,
    /// Warning indicator — top of the volume bar (over-100%) and the FFT
    /// bar gradient peaks.
    pub warn: Color,
    /// Low-contrast text for footer hints and metadata lines.
    pub dim: Color,
    /// Normal body text — non-selected playlist/browser entries and menu
    /// values. A readable, bright neutral that respects the theme palette.
    pub text: Color,
    /// Background fill for every panel.
    pub bg: Color,
}

pub const DEFAULT: Theme = Theme {
    name: "default",
    label: "Default Neon",
    primary: Color::Rgb(255, 89, 194),
    accent: Color::Rgb(0, 229, 255),
    secondary: Color::Rgb(170, 102, 255),
    highlight: Color::Rgb(255, 217, 102),
    ok: Color::Rgb(102, 255, 178),
    warn: Color::Rgb(255, 90, 120),
    dim: Color::Rgb(180, 180, 200),
    text: Color::Rgb(224, 224, 240),
    bg: Color::Rgb(20, 20, 35),
};

pub const MATRIX: Theme = Theme {
    name: "matrix",
    label: "Matrix",
    primary: Color::Rgb(0, 255, 102),
    accent: Color::Rgb(140, 255, 180),
    secondary: Color::Rgb(0, 170, 80),
    highlight: Color::Rgb(200, 255, 200),
    ok: Color::Rgb(0, 220, 100),
    warn: Color::Rgb(255, 100, 100),
    dim: Color::Rgb(110, 170, 120),
    text: Color::Rgb(180, 235, 190),
    bg: Color::Rgb(5, 15, 8),
};

pub const AMBER: Theme = Theme {
    name: "amber",
    label: "Amber CRT",
    primary: Color::Rgb(255, 176, 0),
    accent: Color::Rgb(255, 220, 100),
    secondary: Color::Rgb(200, 130, 30),
    highlight: Color::Rgb(255, 240, 180),
    ok: Color::Rgb(255, 200, 50),
    warn: Color::Rgb(255, 80, 0),
    dim: Color::Rgb(170, 130, 60),
    text: Color::Rgb(255, 238, 200),
    bg: Color::Rgb(25, 15, 5),
};

pub const OCEAN: Theme = Theme {
    name: "ocean",
    label: "Deep Ocean",
    primary: Color::Rgb(0, 180, 255),
    accent: Color::Rgb(120, 220, 255),
    secondary: Color::Rgb(80, 130, 220),
    highlight: Color::Rgb(255, 255, 200),
    ok: Color::Rgb(100, 230, 200),
    warn: Color::Rgb(255, 120, 100),
    dim: Color::Rgb(140, 180, 220),
    text: Color::Rgb(205, 226, 245),
    bg: Color::Rgb(5, 15, 35),
};

pub const MONOCHROME: Theme = Theme {
    name: "monochrome",
    label: "Monochrome",
    primary: Color::Rgb(230, 230, 230),
    accent: Color::Rgb(200, 200, 200),
    secondary: Color::Rgb(140, 140, 140),
    highlight: Color::Rgb(255, 255, 255),
    ok: Color::Rgb(220, 220, 220),
    warn: Color::Rgb(255, 120, 120),
    dim: Color::Rgb(150, 150, 150),
    text: Color::Rgb(225, 225, 225),
    bg: Color::Rgb(15, 15, 15),
};

// === Themes sourced from terminalcolors.com ===
// Each theme is mapped from the source palette as: primary=brMagenta,
// accent=brCyan, secondary=magenta, highlight=brYellow, ok=brGreen,
// warn=brRed, dim=brBlack (falling back to fg when brBlack would be
// unreadable against bg), bg=bg.

pub const APPRENTICE: Theme = Theme {
    name: "apprentice",
    label: "Apprentice",
    primary: Color::Rgb(135, 135, 175),
    accent: Color::Rgb(95, 175, 175),
    secondary: Color::Rgb(95, 95, 135),
    highlight: Color::Rgb(255, 255, 175),
    ok: Color::Rgb(135, 175, 135),
    warn: Color::Rgb(255, 135, 0),
    dim: Color::Rgb(188, 188, 188),
    text: Color::Rgb(200, 200, 200),
    bg: Color::Rgb(38, 38, 38),
};

pub const AYU_DARK: Theme = Theme {
    name: "ayu-dark",
    label: "Ayu Dark",
    primary: Color::Rgb(210, 166, 255),
    accent: Color::Rgb(149, 230, 203),
    secondary: Color::Rgb(205, 161, 250),
    highlight: Color::Rgb(255, 180, 84),
    ok: Color::Rgb(170, 217, 76),
    warn: Color::Rgb(240, 113, 120),
    dim: Color::Rgb(104, 104, 104),
    text: Color::Rgb(191, 189, 182),
    bg: Color::Rgb(11, 14, 20),
};

pub const CATPPUCCIN_MOCHA: Theme = Theme {
    name: "catppuccin-mocha",
    label: "Catppuccin Mocha",
    primary: Color::Rgb(242, 174, 222),
    accent: Color::Rgb(107, 215, 202),
    secondary: Color::Rgb(245, 194, 231),
    highlight: Color::Rgb(235, 211, 145),
    ok: Color::Rgb(137, 216, 139),
    warn: Color::Rgb(243, 119, 153),
    dim: Color::Rgb(88, 91, 112),
    text: Color::Rgb(205, 214, 244),
    bg: Color::Rgb(30, 30, 46),
};

pub const COBALT2: Theme = Theme {
    name: "cobalt2",
    label: "Cobalt2",
    primary: Color::Rgb(251, 148, 255),
    accent: Color::Rgb(128, 252, 255),
    secondary: Color::Rgb(251, 148, 255),
    highlight: Color::Rgb(255, 198, 0),
    ok: Color::Rgb(58, 217, 0),
    warn: Color::Rgb(255, 98, 140),
    dim: Color::Rgb(255, 255, 255),
    text: Color::Rgb(230, 235, 240),
    bg: Color::Rgb(18, 39, 56),
};

pub const DEUS: Theme = Theme {
    name: "deus",
    label: "Deus",
    primary: Color::Rgb(200, 88, 233),
    accent: Color::Rgb(43, 206, 194),
    secondary: Color::Rgb(198, 120, 221),
    highlight: Color::Rgb(237, 191, 105),
    ok: Color::Rgb(144, 201, 102),
    warn: Color::Rgb(236, 62, 69),
    dim: Color::Rgb(234, 234, 234),
    text: Color::Rgb(224, 224, 224),
    bg: Color::Rgb(44, 50, 59),
};

pub const DRACULA: Theme = Theme {
    name: "dracula",
    label: "Dracula",
    primary: Color::Rgb(255, 146, 223),
    accent: Color::Rgb(164, 255, 255),
    secondary: Color::Rgb(255, 121, 198),
    highlight: Color::Rgb(255, 255, 165),
    ok: Color::Rgb(105, 255, 148),
    warn: Color::Rgb(255, 110, 110),
    dim: Color::Rgb(98, 114, 164),
    text: Color::Rgb(248, 248, 242),
    bg: Color::Rgb(40, 42, 54),
};

pub const EVERFOREST_DARK: Theme = Theme {
    name: "everforest-dark",
    label: "Everforest Dark",
    primary: Color::Rgb(214, 153, 182),
    accent: Color::Rgb(131, 192, 146),
    secondary: Color::Rgb(214, 153, 182),
    highlight: Color::Rgb(219, 188, 127),
    ok: Color::Rgb(167, 192, 128),
    warn: Color::Rgb(230, 126, 128),
    dim: Color::Rgb(133, 146, 137),
    text: Color::Rgb(211, 198, 170),
    bg: Color::Rgb(45, 53, 59),
};

pub const GITHUB_DARK: Theme = Theme {
    name: "github-dark",
    label: "GitHub Dark",
    primary: Color::Rgb(210, 168, 255),
    accent: Color::Rgb(86, 212, 221),
    secondary: Color::Rgb(188, 140, 255),
    highlight: Color::Rgb(227, 179, 65),
    ok: Color::Rgb(86, 211, 100),
    warn: Color::Rgb(255, 161, 152),
    dim: Color::Rgb(110, 118, 129),
    text: Color::Rgb(201, 209, 217),
    bg: Color::Rgb(1, 4, 9),
};

pub const GOTHAM: Theme = Theme {
    name: "gotham",
    label: "Gotham",
    primary: Color::Rgb(78, 81, 102),
    accent: Color::Rgb(51, 133, 158),
    secondary: Color::Rgb(78, 81, 102),
    highlight: Color::Rgb(237, 180, 67),
    ok: Color::Rgb(42, 168, 137),
    warn: Color::Rgb(194, 49, 39),
    dim: Color::Rgb(153, 209, 206),
    text: Color::Rgb(197, 209, 207),
    bg: Color::Rgb(12, 16, 20),
};

pub const GRUVBOX_DARK: Theme = Theme {
    name: "gruvbox-dark",
    label: "Gruvbox Dark",
    primary: Color::Rgb(211, 134, 155),
    accent: Color::Rgb(142, 192, 124),
    secondary: Color::Rgb(177, 98, 134),
    highlight: Color::Rgb(250, 189, 47),
    ok: Color::Rgb(184, 187, 38),
    warn: Color::Rgb(251, 73, 52),
    dim: Color::Rgb(146, 131, 116),
    text: Color::Rgb(235, 219, 178),
    bg: Color::Rgb(40, 40, 40),
};

pub const ICEBERG_DARK: Theme = Theme {
    name: "iceberg-dark",
    label: "Iceberg Dark",
    primary: Color::Rgb(173, 160, 211),
    accent: Color::Rgb(149, 196, 206),
    secondary: Color::Rgb(160, 147, 199),
    highlight: Color::Rgb(233, 177, 137),
    ok: Color::Rgb(192, 202, 142),
    warn: Color::Rgb(233, 137, 137),
    dim: Color::Rgb(107, 112, 137),
    text: Color::Rgb(198, 203, 222),
    bg: Color::Rgb(22, 24, 33),
};

pub const JELLYBEANS: Theme = Theme {
    name: "jellybeans",
    label: "Jellybeans",
    primary: Color::Rgb(251, 218, 255),
    accent: Color::Rgb(26, 178, 168),
    secondary: Color::Rgb(225, 192, 250),
    highlight: Color::Rgb(255, 220, 160),
    ok: Color::Rgb(189, 222, 171),
    warn: Color::Rgb(255, 161, 161),
    dim: Color::Rgb(189, 189, 189),
    text: Color::Rgb(220, 220, 220),
    bg: Color::Rgb(18, 18, 18),
};

pub const KANAGAWA_WAVE: Theme = Theme {
    name: "kanagawa-wave",
    label: "Kanagawa Wave",
    primary: Color::Rgb(147, 138, 169),
    accent: Color::Rgb(122, 168, 159),
    secondary: Color::Rgb(149, 127, 184),
    highlight: Color::Rgb(230, 195, 132),
    ok: Color::Rgb(152, 187, 108),
    warn: Color::Rgb(232, 36, 36),
    dim: Color::Rgb(114, 113, 105),
    text: Color::Rgb(220, 215, 186),
    bg: Color::Rgb(31, 31, 40),
};

pub const LUCARIO: Theme = Theme {
    name: "lucario",
    label: "Lucario",
    primary: Color::Rgb(212, 169, 255),
    accent: Color::Rgb(185, 236, 253),
    secondary: Color::Rgb(202, 148, 255),
    highlight: Color::Rgb(255, 255, 165),
    ok: Color::Rgb(114, 204, 90),
    warn: Color::Rgb(255, 101, 65),
    dim: Color::Rgb(248, 248, 242),
    text: Color::Rgb(230, 238, 243),
    bg: Color::Rgb(43, 62, 80),
};

pub const MIASMA: Theme = Theme {
    name: "miasma",
    label: "Miasma",
    primary: Color::Rgb(187, 119, 68),
    accent: Color::Rgb(201, 165, 84),
    secondary: Color::Rgb(187, 119, 68),
    highlight: Color::Rgb(179, 109, 67),
    ok: Color::Rgb(95, 135, 95),
    warn: Color::Rgb(104, 87, 66),
    dim: Color::Rgb(102, 102, 102),
    text: Color::Rgb(202, 192, 157),
    bg: Color::Rgb(34, 34, 34),
};

pub const MOONFLY: Theme = Theme {
    name: "moonfly",
    label: "Moonfly",
    primary: Color::Rgb(174, 129, 255),
    accent: Color::Rgb(133, 220, 133),
    secondary: Color::Rgb(207, 135, 232),
    highlight: Color::Rgb(198, 198, 132),
    ok: Color::Rgb(54, 198, 146),
    warn: Color::Rgb(255, 81, 137),
    dim: Color::Rgb(148, 148, 148),
    text: Color::Rgb(205, 205, 205),
    bg: Color::Rgb(8, 8, 8),
};

pub const NIGHT_OWL_DARK: Theme = Theme {
    name: "night-owl-dark",
    label: "Night Owl",
    primary: Color::Rgb(199, 146, 234),
    accent: Color::Rgb(127, 219, 202),
    secondary: Color::Rgb(199, 146, 234),
    highlight: Color::Rgb(255, 235, 149),
    ok: Color::Rgb(34, 218, 110),
    warn: Color::Rgb(239, 83, 80),
    dim: Color::Rgb(87, 86, 86),
    text: Color::Rgb(214, 222, 235),
    bg: Color::Rgb(1, 22, 39),
};

pub const NIGHTFLY: Theme = Theme {
    name: "nightfly",
    label: "Nightfly",
    primary: Color::Rgb(174, 129, 255),
    accent: Color::Rgb(127, 219, 202),
    secondary: Color::Rgb(199, 146, 234),
    highlight: Color::Rgb(236, 196, 141),
    ok: Color::Rgb(33, 199, 168),
    warn: Color::Rgb(255, 88, 116),
    dim: Color::Rgb(124, 143, 143),
    text: Color::Rgb(200, 210, 222),
    bg: Color::Rgb(1, 22, 39),
};

pub const NIGHTFOX: Theme = Theme {
    name: "nightfox",
    label: "Nightfox",
    primary: Color::Rgb(186, 161, 226),
    accent: Color::Rgb(122, 213, 214),
    secondary: Color::Rgb(157, 121, 214),
    highlight: Color::Rgb(224, 201, 137),
    ok: Color::Rgb(142, 186, 164),
    warn: Color::Rgb(209, 105, 131),
    dim: Color::Rgb(205, 206, 207),
    text: Color::Rgb(205, 206, 207),
    bg: Color::Rgb(25, 35, 48),
};

pub const NOCTIS: Theme = Theme {
    name: "noctis",
    label: "Noctis",
    primary: Color::Rgb(231, 152, 179),
    accent: Color::Rgb(96, 219, 235),
    secondary: Color::Rgb(223, 118, 155),
    highlight: Color::Rgb(230, 149, 51),
    ok: Color::Rgb(96, 235, 177),
    warn: Color::Rgb(233, 119, 73),
    dim: Color::Rgb(71, 104, 108),
    text: Color::Rgb(179, 201, 204),
    bg: Color::Rgb(3, 25, 27),
};

pub const NORD: Theme = Theme {
    name: "nord",
    label: "Nord",
    primary: Color::Rgb(180, 142, 173),
    accent: Color::Rgb(143, 188, 187),
    secondary: Color::Rgb(180, 142, 173),
    highlight: Color::Rgb(235, 203, 139),
    ok: Color::Rgb(163, 190, 140),
    warn: Color::Rgb(191, 97, 106),
    dim: Color::Rgb(216, 222, 233),
    text: Color::Rgb(216, 222, 233),
    bg: Color::Rgb(46, 52, 64),
};

pub const NORDIC: Theme = Theme {
    name: "nordic",
    label: "Nordic",
    primary: Color::Rgb(190, 157, 136),
    accent: Color::Rgb(159, 198, 197),
    secondary: Color::Rgb(180, 142, 173),
    highlight: Color::Rgb(239, 212, 159),
    ok: Color::Rgb(177, 200, 157),
    warn: Color::Rgb(197, 114, 122),
    dim: Color::Rgb(187, 195, 212),
    text: Color::Rgb(201, 209, 222),
    bg: Color::Rgb(36, 41, 51),
};

pub const ONE_DARK: Theme = Theme {
    name: "one-dark",
    label: "One Dark",
    primary: Color::Rgb(198, 120, 221),
    accent: Color::Rgb(86, 182, 194),
    secondary: Color::Rgb(198, 120, 221),
    highlight: Color::Rgb(209, 154, 102),
    ok: Color::Rgb(152, 195, 121),
    warn: Color::Rgb(224, 108, 117),
    dim: Color::Rgb(171, 178, 191),
    text: Color::Rgb(190, 197, 209),
    bg: Color::Rgb(40, 44, 52),
};

pub const ONE_HALF_DARK: Theme = Theme {
    name: "one-half-dark",
    label: "One Half Dark",
    primary: Color::Rgb(198, 120, 221),
    accent: Color::Rgb(86, 182, 194),
    secondary: Color::Rgb(198, 120, 221),
    highlight: Color::Rgb(229, 192, 123),
    ok: Color::Rgb(152, 195, 121),
    warn: Color::Rgb(224, 108, 117),
    dim: Color::Rgb(220, 223, 228),
    text: Color::Rgb(220, 223, 228),
    bg: Color::Rgb(40, 44, 52),
};

pub const PANDA: Theme = Theme {
    name: "panda",
    label: "Panda",
    primary: Color::Rgb(255, 154, 193),
    accent: Color::Rgb(188, 170, 254),
    secondary: Color::Rgb(255, 117, 181),
    highlight: Color::Rgb(255, 204, 149),
    ok: Color::Rgb(25, 249, 216),
    warn: Color::Rgb(255, 44, 109),
    dim: Color::Rgb(117, 117, 117),
    text: Color::Rgb(230, 230, 235),
    bg: Color::Rgb(41, 42, 43),
};

pub const POSTERPOLE: Theme = Theme {
    name: "posterpole",
    label: "Posterpole",
    primary: Color::Rgb(204, 179, 198),
    accent: Color::Rgb(170, 187, 186),
    secondary: Color::Rgb(184, 148, 175),
    highlight: Color::Rgb(217, 172, 140),
    ok: Color::Rgb(146, 163, 143),
    warn: Color::Rgb(188, 143, 143),
    dim: Color::Rgb(165, 165, 156),
    text: Color::Rgb(212, 207, 200),
    bg: Color::Rgb(37, 34, 42),
};

pub const ROSE_PINE: Theme = Theme {
    name: "rose-pine",
    label: "Rosé Pine",
    primary: Color::Rgb(196, 167, 231),
    accent: Color::Rgb(235, 188, 186),
    secondary: Color::Rgb(196, 167, 231),
    highlight: Color::Rgb(246, 193, 119),
    ok: Color::Rgb(49, 116, 143),
    warn: Color::Rgb(235, 111, 146),
    dim: Color::Rgb(144, 140, 170),
    text: Color::Rgb(224, 222, 244),
    bg: Color::Rgb(31, 29, 46),
};

pub const SEOUL256_DARK: Theme = Theme {
    name: "seoul256-dark",
    label: "Seoul256 Dark",
    primary: Color::Rgb(255, 175, 175),
    accent: Color::Rgb(135, 215, 215),
    secondary: Color::Rgb(215, 175, 175),
    highlight: Color::Rgb(255, 215, 135),
    ok: Color::Rgb(135, 175, 135),
    warn: Color::Rgb(215, 95, 135),
    dim: Color::Rgb(208, 208, 208),
    text: Color::Rgb(208, 208, 208),
    bg: Color::Rgb(58, 58, 58),
};

pub const SHADES_OF_PURPLE: Theme = Theme {
    name: "shades-of-purple",
    label: "Shades of Purple",
    primary: Color::Rgb(251, 148, 255),
    accent: Color::Rgb(128, 252, 255),
    secondary: Color::Rgb(255, 44, 112),
    highlight: Color::Rgb(250, 208, 0),
    ok: Color::Rgb(58, 217, 0),
    warn: Color::Rgb(228, 57, 55),
    dim: Color::Rgb(255, 255, 255),
    text: Color::Rgb(230, 230, 245),
    bg: Color::Rgb(30, 30, 63),
};

pub const SOLARIZED_DARK: Theme = Theme {
    name: "solarized-dark",
    label: "Solarized Dark",
    primary: Color::Rgb(108, 113, 196),
    accent: Color::Rgb(147, 161, 161),
    secondary: Color::Rgb(211, 54, 130),
    highlight: Color::Rgb(101, 123, 131),
    ok: Color::Rgb(88, 110, 117),
    warn: Color::Rgb(203, 75, 22),
    dim: Color::Rgb(131, 148, 150),
    text: Color::Rgb(147, 161, 161),
    bg: Color::Rgb(0, 43, 54),
};

pub const SONOKAI: Theme = Theme {
    name: "sonokai",
    label: "Sonokai",
    primary: Color::Rgb(179, 157, 243),
    accent: Color::Rgb(243, 150, 96),
    secondary: Color::Rgb(179, 157, 243),
    highlight: Color::Rgb(231, 198, 100),
    ok: Color::Rgb(158, 208, 114),
    warn: Color::Rgb(252, 93, 124),
    dim: Color::Rgb(127, 132, 144),
    text: Color::Rgb(224, 224, 226),
    bg: Color::Rgb(44, 46, 52),
};

pub const SRCERY: Theme = Theme {
    name: "srcery",
    label: "Srcery",
    primary: Color::Rgb(255, 92, 143),
    accent: Color::Rgb(43, 228, 208),
    secondary: Color::Rgb(224, 44, 109),
    highlight: Color::Rgb(254, 208, 110),
    ok: Color::Rgb(152, 188, 55),
    warn: Color::Rgb(247, 83, 65),
    dim: Color::Rgb(145, 129, 117),
    text: Color::Rgb(251, 241, 199),
    bg: Color::Rgb(28, 27, 25),
};

pub const TENDER: Theme = Theme {
    name: "tender",
    label: "Tender",
    primary: Color::Rgb(211, 185, 135),
    accent: Color::Rgb(115, 206, 244),
    secondary: Color::Rgb(211, 185, 135),
    highlight: Color::Rgb(255, 194, 75),
    ok: Color::Rgb(201, 208, 92),
    warn: Color::Rgb(244, 55, 83),
    dim: Color::Rgb(238, 238, 238),
    text: Color::Rgb(224, 224, 224),
    bg: Color::Rgb(40, 40, 40),
};

pub const TOKYO_NIGHT: Theme = Theme {
    name: "tokyo-night",
    label: "Tokyo Night",
    primary: Color::Rgb(187, 154, 247),
    accent: Color::Rgb(125, 207, 255),
    secondary: Color::Rgb(187, 154, 247),
    highlight: Color::Rgb(224, 175, 104),
    ok: Color::Rgb(158, 206, 106),
    warn: Color::Rgb(247, 118, 142),
    dim: Color::Rgb(192, 202, 245),
    text: Color::Rgb(192, 202, 245),
    bg: Color::Rgb(26, 27, 38),
};

pub const TOMORROW_NIGHT: Theme = Theme {
    name: "tomorrow-night",
    label: "Tomorrow Night",
    primary: Color::Rgb(178, 148, 187),
    accent: Color::Rgb(138, 190, 183),
    secondary: Color::Rgb(178, 148, 187),
    highlight: Color::Rgb(240, 198, 116),
    ok: Color::Rgb(181, 189, 104),
    warn: Color::Rgb(204, 102, 102),
    dim: Color::Rgb(197, 200, 198),
    text: Color::Rgb(197, 200, 198),
    bg: Color::Rgb(29, 31, 33),
};

pub const ZENBONES_ZENWRITTEN_DARK: Theme = Theme {
    name: "zenbones-zenwritten-dark",
    label: "Zenbones Zenwritten Dark",
    primary: Color::Rgb(207, 134, 193),
    accent: Color::Rgb(101, 184, 193),
    secondary: Color::Rgb(178, 121, 167),
    highlight: Color::Rgb(214, 140, 103),
    ok: Color::Rgb(139, 174, 104),
    warn: Color::Rgb(232, 131, 143),
    dim: Color::Rgb(187, 187, 187),
    text: Color::Rgb(210, 210, 205),
    bg: Color::Rgb(25, 25, 25),
};

pub const ALL: &[Theme] = &[
    DEFAULT, MATRIX, AMBER, OCEAN, MONOCHROME,
    APPRENTICE, AYU_DARK, CATPPUCCIN_MOCHA, COBALT2, DEUS, DRACULA,
    EVERFOREST_DARK, GITHUB_DARK, GOTHAM, GRUVBOX_DARK, ICEBERG_DARK,
    JELLYBEANS, KANAGAWA_WAVE, LUCARIO, MIASMA, MOONFLY, NIGHT_OWL_DARK,
    NIGHTFLY, NIGHTFOX, NOCTIS, NORD, NORDIC, ONE_DARK, ONE_HALF_DARK,
    PANDA, POSTERPOLE, ROSE_PINE, SEOUL256_DARK, SHADES_OF_PURPLE,
    SOLARIZED_DARK, SONOKAI, SRCERY, TENDER, TOKYO_NIGHT, TOMORROW_NIGHT,
    ZENBONES_ZENWRITTEN_DARK,
];

pub fn by_name(name: &str) -> Theme {
    ALL.iter()
        .copied()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .unwrap_or(DEFAULT)
}

pub fn next_after(name: &str) -> Theme {
    let idx = ALL.iter().position(|t| t.name == name).unwrap_or(0);
    ALL[(idx + 1) % ALL.len()]
}

pub fn prev_before(name: &str) -> Theme {
    let idx = ALL.iter().position(|t| t.name == name).unwrap_or(0);
    ALL[(idx + ALL.len() - 1) % ALL.len()]
}

thread_local! {
    static CURRENT: Cell<Theme> = const { Cell::new(DEFAULT) };
}

pub fn current() -> Theme {
    CURRENT.with(|c| c.get())
}

pub fn set_current(theme: Theme) {
    CURRENT.with(|c| c.set(theme));
}

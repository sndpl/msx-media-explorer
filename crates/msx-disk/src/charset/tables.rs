//! Byte->Unicode mapping tables for the MSX character sets.
//!
//! Each table covers the high half (`0x80..=0xFF`) of one MSX code page; index
//! `i` corresponds to byte `0x80 + i`. Sourced from the MSX Technical Data Book
//! and the Unicode L2/19-025 proposal (MSX.TXT). `U+FFFD` marks cells that are
//! genuinely undefined on real hardware. `0xFF` is the hardware text cursor,
//! rendered here as a full block (`U+2588`).

/// MSX International (Western) high half, `0x80..=0xFF`.
pub static INTERNATIONAL_HIGH: [char; 128] = [
    // 0x8x
    '\u{00C7}',
    '\u{00FC}',
    '\u{00E9}',
    '\u{00E2}',
    '\u{00E4}',
    '\u{00E0}',
    '\u{00E5}',
    '\u{00E7}',
    '\u{00EA}',
    '\u{00EB}',
    '\u{00E8}',
    '\u{00EF}',
    '\u{00EE}',
    '\u{00EC}',
    '\u{00C4}',
    '\u{00C5}',
    // 0x9x
    '\u{00C9}',
    '\u{00E6}',
    '\u{00C6}',
    '\u{00F4}',
    '\u{00F6}',
    '\u{00F2}',
    '\u{00FB}',
    '\u{00F9}',
    '\u{00FF}',
    '\u{00D6}',
    '\u{00DC}',
    '\u{00A2}',
    '\u{00A3}',
    '\u{00A5}',
    '\u{20A7}',
    '\u{0192}',
    // 0xAx
    '\u{00E1}',
    '\u{00ED}',
    '\u{00F3}',
    '\u{00FA}',
    '\u{00F1}',
    '\u{00D1}',
    '\u{00AA}',
    '\u{00BA}',
    '\u{00BF}',
    '\u{2310}',
    '\u{00AC}',
    '\u{00BD}',
    '\u{00BC}',
    '\u{00A1}',
    '\u{00AB}',
    '\u{00BB}',
    // 0xBx
    '\u{00C3}',
    '\u{00E3}',
    '\u{0128}',
    '\u{0129}',
    '\u{00D5}',
    '\u{00F5}',
    '\u{0170}',
    '\u{0171}',
    '\u{0132}',
    '\u{0133}',
    '\u{00BE}',
    '\u{223D}',
    '\u{25CA}',
    '\u{2030}',
    '\u{00B6}',
    '\u{00A7}',
    // 0xCx (several are Symbols for Legacy Computing, U+1FBxx, outside the BMP)
    '\u{2582}',
    '\u{259A}',
    '\u{2586}',
    '\u{1FB82}',
    '\u{25AC}',
    '\u{1FB85}',
    '\u{258E}',
    '\u{259E}',
    '\u{258A}',
    '\u{1FB87}',
    '\u{1FB8A}',
    '\u{1FB99}',
    '\u{1FB98}',
    '\u{1FB6D}',
    '\u{1FB6F}',
    '\u{1FB6C}',
    // 0xDx
    '\u{1FB6E}',
    '\u{1FB9A}',
    '\u{1FB9B}',
    '\u{2598}',
    '\u{2597}',
    '\u{259D}',
    '\u{2596}',
    '\u{1FB96}',
    '\u{0394}',
    '\u{2021}',
    '\u{03C9}',
    '\u{2588}',
    '\u{2584}',
    '\u{258C}',
    '\u{2590}',
    '\u{2580}',
    // 0xEx
    '\u{03B1}',
    '\u{00DF}',
    '\u{0393}',
    '\u{03C0}',
    '\u{03A3}',
    '\u{03C3}',
    '\u{00B5}',
    '\u{03C4}',
    '\u{03A6}',
    '\u{0398}',
    '\u{03A9}',
    '\u{03B4}',
    '\u{221E}',
    '\u{2300}',
    '\u{2208}',
    '\u{2229}',
    // 0xFx (0xFF is the cursor; rendered as a full block)
    '\u{2261}',
    '\u{00B1}',
    '\u{2265}',
    '\u{2264}',
    '\u{2320}',
    '\u{2321}',
    '\u{00F7}',
    '\u{2248}',
    '\u{00B0}',
    '\u{2219}',
    '\u{00B7}',
    '\u{221A}',
    '\u{207F}',
    '\u{00B2}',
    '\u{25A0}',
    '\u{2588}',
];

/// MSX Japanese high half, `0x80..=0xFF`: hiragana, JIS X 0201 half-width
/// katakana, card suits and a couple of graphic glyphs.
pub static JAPANESE_HIGH: [char; 128] = [
    // 0x8x
    '\u{2660}', '\u{2665}', '\u{2663}', '\u{2666}', '\u{25CB}', '\u{25CF}', '\u{3092}', '\u{3041}',
    '\u{3043}', '\u{3045}', '\u{3047}', '\u{3049}', '\u{3083}', '\u{3085}', '\u{3087}', '\u{3063}',
    // 0x9x (0x90 undefined)
    '\u{FFFD}', '\u{3042}', '\u{3044}', '\u{3046}', '\u{3048}', '\u{304A}', '\u{304B}', '\u{304D}',
    '\u{304F}', '\u{3051}', '\u{3053}', '\u{3055}', '\u{3057}', '\u{3059}', '\u{305B}', '\u{305D}',
    // 0xAx (0xA0 undefined; 0xA1.. = JIS X 0201 katakana punctuation/letters)
    '\u{FFFD}', '\u{FF61}', '\u{FF62}', '\u{FF63}', '\u{FF64}', '\u{FF65}', '\u{FF66}', '\u{FF67}',
    '\u{FF68}', '\u{FF69}', '\u{FF6A}', '\u{FF6B}', '\u{FF6C}', '\u{FF6D}', '\u{FF6E}', '\u{FF6F}',
    // 0xBx
    '\u{FF70}', '\u{FF71}', '\u{FF72}', '\u{FF73}', '\u{FF74}', '\u{FF75}', '\u{FF76}', '\u{FF77}',
    '\u{FF78}', '\u{FF79}', '\u{FF7A}', '\u{FF7B}', '\u{FF7C}', '\u{FF7D}', '\u{FF7E}', '\u{FF7F}',
    // 0xCx
    '\u{FF80}', '\u{FF81}', '\u{FF82}', '\u{FF83}', '\u{FF84}', '\u{FF85}', '\u{FF86}', '\u{FF87}',
    '\u{FF88}', '\u{FF89}', '\u{FF8A}', '\u{FF8B}', '\u{FF8C}', '\u{FF8D}', '\u{FF8E}', '\u{FF8F}',
    // 0xDx
    '\u{FF90}', '\u{FF91}', '\u{FF92}', '\u{FF93}', '\u{FF94}', '\u{FF95}', '\u{FF96}', '\u{FF97}',
    '\u{FF98}', '\u{FF99}', '\u{FF9A}', '\u{FF9B}', '\u{FF9C}', '\u{FF9D}', '\u{FF9E}', '\u{FF9F}',
    // 0xEx
    '\u{305F}', '\u{3061}', '\u{3064}', '\u{3066}', '\u{3068}', '\u{306A}', '\u{306B}', '\u{306C}',
    '\u{306D}', '\u{306E}', '\u{306F}', '\u{3072}', '\u{3075}', '\u{3078}', '\u{307B}', '\u{307E}',
    // 0xFx (0xFE undefined; 0xFF is the cursor, rendered as a full block)
    '\u{307F}', '\u{3080}', '\u{3081}', '\u{3082}', '\u{3084}', '\u{3086}', '\u{3088}', '\u{3089}',
    '\u{308A}', '\u{308B}', '\u{308C}', '\u{308D}', '\u{308F}', '\u{3093}', '\u{FFFD}', '\u{2588}',
];

// The four regional tables below are transcribed from the current KreativeKorp
// MSX *Video* mappings (`msxvid{ru,kr,ar,br}.txt`, github.com/kreativekorp/charset),
// the same family of files behind [`INTERNATIONAL_HIGH`]/[`JAPANESE_HIGH`]. As with
// those, `0xFF` is the hardware text cursor and is forced to a full block
// (`U+2588`) even though the source files leave it undefined. A few obscure
// semigraphic cells differ slightly from the older International/Japanese tables
// above (the source data was revised after those were committed); the existing
// tables are kept as-is because their kana mapping is validated against real data.

/// MSX Russian high half, `0x80..=0xFF`: relocated legacy/box-drawing graphics
/// and Greek/math symbols (`0x80..=0xBF`) followed by Cyrillic (`0xC0..=0xFE`).
pub static RUSSIAN_HIGH: [char; 128] = [
    // 0x80
    '\u{2582}',
    '\u{259A}',
    '\u{2586}',
    '\u{1FB82}',
    '\u{25AC}',
    '\u{1FB85}',
    '\u{258E}',
    '\u{259E}',
    // 0x88
    '\u{258A}',
    '\u{1FB87}',
    '\u{1FB8A}',
    '\u{1FB99}',
    '\u{1FB98}',
    '\u{1FB6D}',
    '\u{1FB6F}',
    '\u{1FB6C}',
    // 0x90
    '\u{1FB6E}',
    '\u{1FB9A}',
    '\u{1FB9B}',
    '\u{2598}',
    '\u{2597}',
    '\u{259D}',
    '\u{2596}',
    '\u{1FB96}',
    // 0x98
    '\u{0394}',
    '\u{2021}',
    '\u{03C9}',
    '\u{2588}',
    '\u{2584}',
    '\u{258C}',
    '\u{2590}',
    '\u{2580}',
    // 0xA0
    '\u{03B1}',
    '\u{00DF}',
    '\u{0393}',
    '\u{03C0}',
    '\u{03A3}',
    '\u{03C3}',
    '\u{00B5}',
    '\u{03C4}',
    // 0xA8
    '\u{03A6}',
    '\u{0398}',
    '\u{03A9}',
    '\u{03B4}',
    '\u{221E}',
    '\u{2205}',
    '\u{2208}',
    '\u{2229}',
    // 0xB0
    '\u{2261}',
    '\u{00B1}',
    '\u{2265}',
    '\u{2264}',
    '\u{2320}',
    '\u{2321}',
    '\u{00F7}',
    '\u{2248}',
    // 0xB8
    '\u{00B0}',
    '\u{2219}',
    '\u{00B7}',
    '\u{221A}',
    '\u{207F}',
    '\u{00B2}',
    '\u{25A0}',
    '\u{00A4}',
    // 0xC0
    '\u{044E}',
    '\u{0430}',
    '\u{0431}',
    '\u{0446}',
    '\u{0434}',
    '\u{0435}',
    '\u{0444}',
    '\u{0433}',
    // 0xC8
    '\u{0445}',
    '\u{0438}',
    '\u{0439}',
    '\u{043A}',
    '\u{043B}',
    '\u{043C}',
    '\u{043D}',
    '\u{043E}',
    // 0xD0
    '\u{043F}',
    '\u{044F}',
    '\u{0440}',
    '\u{0441}',
    '\u{0442}',
    '\u{0443}',
    '\u{0436}',
    '\u{0432}',
    // 0xD8
    '\u{044C}',
    '\u{044B}',
    '\u{0437}',
    '\u{0448}',
    '\u{044D}',
    '\u{0449}',
    '\u{0447}',
    '\u{044A}',
    // 0xE0
    '\u{042E}',
    '\u{0410}',
    '\u{0411}',
    '\u{0426}',
    '\u{0414}',
    '\u{0415}',
    '\u{0424}',
    '\u{0413}',
    // 0xE8
    '\u{0425}',
    '\u{0418}',
    '\u{0419}',
    '\u{041A}',
    '\u{041B}',
    '\u{041C}',
    '\u{041D}',
    '\u{041E}',
    // 0xF0
    '\u{041F}',
    '\u{042F}',
    '\u{0420}',
    '\u{0421}',
    '\u{0422}',
    '\u{0423}',
    '\u{0416}',
    '\u{0412}',
    // 0xF8 (0xFF is the cursor, rendered as a full block)
    '\u{042C}',
    '\u{042B}',
    '\u{0417}',
    '\u{0428}',
    '\u{042D}',
    '\u{0429}',
    '\u{0427}',
    '\u{2588}',
];

/// MSX Korean high half, `0x80..=0xFF`: card suits, Hangul compatibility jamo
/// (`U+3131..`) and a block of precomposed Hangul syllables.
pub static KOREAN_HIGH: [char; 128] = [
    // 0x80
    '\u{2660}', '\u{2665}', '\u{2663}', '\u{2666}', '\u{25CB}', '\u{25CF}', '\u{3131}', '\u{3132}',
    // 0x88
    '\u{3134}', '\u{3137}', '\u{3138}', '\u{3139}', '\u{3141}', '\u{3142}', '\u{3143}', '\u{3145}',
    // 0x90
    '\u{3146}', '\u{3147}', '\u{3148}', '\u{3149}', '\u{314A}', '\u{314B}', '\u{314C}', '\u{314D}',
    // 0x98
    '\u{314E}', '\u{314F}', '\u{3150}', '\u{3151}', '\u{3152}', '\u{3153}', '\u{3154}', '\u{3155}',
    // 0xA0
    '\u{3156}', '\u{3157}', '\u{315B}', '\u{315C}', '\u{3160}', '\u{3161}', '\u{3163}', '\u{ACE0}',
    // 0xA8
    '\u{AD50}', '\u{AD6C}', '\u{ADDC}', '\u{ADF8}', '\u{AF2C}', '\u{AF9C}', '\u{AFB8}', '\u{B028}',
    // 0xB0
    '\u{B044}', '\u{B178}', '\u{B1E8}', '\u{B204}', '\u{B274}', '\u{B290}', '\u{B3C4}', '\u{B434}',
    // 0xB8
    '\u{B450}', '\u{B4C0}', '\u{B4DC}', '\u{B610}', '\u{B680}', '\u{B69C}', '\u{B70C}', '\u{B728}',
    // 0xC0
    '\u{B85C}', '\u{B8CC}', '\u{B8E8}', '\u{B958}', '\u{B974}', '\u{BAA8}', '\u{BB18}', '\u{BB34}',
    // 0xC8
    '\u{BBA4}', '\u{BBC0}', '\u{BCF4}', '\u{BD64}', '\u{BD80}', '\u{BDF0}', '\u{BE0C}', '\u{BF40}',
    // 0xD0
    '\u{BFB0}', '\u{BFCC}', '\u{C03C}', '\u{C058}', '\u{C18C}', '\u{C1FC}', '\u{C218}', '\u{C288}',
    // 0xD8
    '\u{C2A4}', '\u{C3D8}', '\u{C448}', '\u{C464}', '\u{C4D4}', '\u{C4F0}', '\u{C624}', '\u{C694}',
    // 0xE0
    '\u{C6B0}', '\u{C720}', '\u{C73C}', '\u{C870}', '\u{C8E0}', '\u{C8FC}', '\u{C96C}', '\u{C988}',
    // 0xE8
    '\u{CABC}', '\u{CB2C}', '\u{CB48}', '\u{CBB8}', '\u{CBD4}', '\u{CD08}', '\u{CD78}', '\u{CD94}',
    // 0xF0
    '\u{CE04}', '\u{CE20}', '\u{CF54}', '\u{CFC4}', '\u{CFE0}', '\u{D050}', '\u{D06C}', '\u{D1A0}',
    // 0xF8 (0xFC..0xFE undefined; 0xFF is the cursor, rendered as a full block)
    '\u{D210}', '\u{D22C}', '\u{D29C}', '\u{D2B8}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{2588}',
];

/// MSX Arabic high half, `0x80..=0xFF`: Arabic-Indic digits, punctuation and
/// Arabic letters (mix of base letters and isolated/medial/final presentation
/// forms). egui does not run Arabic shaping or RTL reordering, so these render
/// unjoined and left-to-right; cells the source marks as bidi-control or leaves
/// undefined map to `U+FFFD`.
pub static ARABIC_HIGH: [char; 128] = [
    // 0x80
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{066A}', '\u{FFFD}', '\u{FFFD}',
    // 0x88
    '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{060C}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
    // 0x90
    '\u{0660}', '\u{0661}', '\u{0662}', '\u{0663}', '\u{0664}', '\u{0665}', '\u{0666}', '\u{0667}',
    // 0x98
    '\u{0668}', '\u{0669}', '\u{FFFD}', '\u{061B}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{061F}',
    // 0xA0
    '\u{FFFD}', '\u{FE8C}', '\u{0626}', '\u{FE92}', '\u{0628}', '\u{FE98}', '\u{062A}', '\u{FE9C}',
    // 0xA8
    '\u{062B}', '\u{FEA0}', '\u{062C}', '\u{FEA4}', '\u{062D}', '\u{FEA8}', '\u{062E}', '\u{FEB4}',
    // 0xB0
    '\u{0633}', '\u{FEB8}', '\u{0634}', '\u{FEBC}', '\u{0635}', '\u{FEC0}', '\u{0636}', '\u{0637}',
    // 0xB8
    '\u{0638}', '\u{FECB}', '\u{0639}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}',
    // 0xC0
    '\u{FECC}', '\u{FECA}', '\u{FECF}', '\u{063A}', '\u{FED0}', '\u{FECE}', '\u{FED4}', '\u{0641}',
    // 0xC8
    '\u{FED8}', '\u{0642}', '\u{FEDC}', '\u{0643}', '\u{FEE0}', '\u{0644}', '\u{FEE4}', '\u{0645}',
    // 0xD0
    '\u{FEE8}', '\u{0646}', '\u{FEEC}', '\u{0647}', '\u{FEF4}', '\u{064A}', '\u{FEF2}', '\u{0622}',
    // 0xD8
    '\u{FE82}', '\u{0623}', '\u{FE84}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{FFFD}', '\u{0624}',
    // 0xE0
    '\u{0625}', '\u{FE88}', '\u{0627}', '\u{FE8E}', '\u{0629}', '\u{062F}', '\u{0630}', '\u{0631}',
    // 0xE8
    '\u{0632}', '\u{0648}', '\u{0649}', '\u{FEF0}', '\u{FEFC}', '\u{FEF8}', '\u{FEF6}', '\u{FEFA}',
    // 0xF0
    '\u{0621}', '\u{0640}', '\u{064B}', '\u{064C}', '\u{064D}', '\u{064E}', '\u{FE77}', '\u{064F}',
    // 0xF8 (0xFF is the cursor, rendered as a full block)
    '\u{FE79}', '\u{0650}', '\u{FE7B}', '\u{0651}', '\u{FE7D}', '\u{0652}', '\u{FE7F}', '\u{2588}',
];

/// MSX Brazilian high half, `0x80..=0xFF`: the BRASCII/ABNT set — International
/// base with extra Portuguese accents, the Cruzeiro sign at `0x9E`, and the same
/// legacy/box-drawing + Greek/math graphics block as International.
pub static BRAZILIAN_HIGH: [char; 128] = [
    // 0x80
    '\u{00C7}',
    '\u{00FC}',
    '\u{00E9}',
    '\u{00E2}',
    '\u{00C1}',
    '\u{00E0}',
    '\u{00A8}',
    '\u{00E7}',
    // 0x88
    '\u{00EA}',
    '\u{00CD}',
    '\u{00D3}',
    '\u{00DA}',
    '\u{00C2}',
    '\u{00CA}',
    '\u{00D4}',
    '\u{00C0}',
    // 0x90
    '\u{00C9}',
    '\u{00E6}',
    '\u{00C6}',
    '\u{00F4}',
    '\u{00F6}',
    '\u{00F2}',
    '\u{00FB}',
    '\u{00F9}',
    // 0x98
    '\u{00FF}',
    '\u{00D6}',
    '\u{00DC}',
    '\u{00A2}',
    '\u{00A3}',
    '\u{00A5}',
    '\u{20A2}',
    '\u{0192}',
    // 0xA0
    '\u{00E1}',
    '\u{00ED}',
    '\u{00F3}',
    '\u{00FA}',
    '\u{00F1}',
    '\u{00D1}',
    '\u{00AA}',
    '\u{00BA}',
    // 0xA8
    '\u{00BF}',
    '\u{2310}',
    '\u{00AC}',
    '\u{00BD}',
    '\u{00BC}',
    '\u{00A1}',
    '\u{00AB}',
    '\u{00BB}',
    // 0xB0
    '\u{00C3}',
    '\u{00E3}',
    '\u{0128}',
    '\u{0129}',
    '\u{00D5}',
    '\u{00F5}',
    '\u{0168}',
    '\u{0169}',
    // 0xB8
    '\u{0132}',
    '\u{0133}',
    '\u{00BE}',
    '\u{223D}',
    '\u{25C7}',
    '\u{2030}',
    '\u{00B6}',
    '\u{00A7}',
    // 0xC0
    '\u{2582}',
    '\u{259A}',
    '\u{2586}',
    '\u{1FB82}',
    '\u{25AC}',
    '\u{1FB85}',
    '\u{258E}',
    '\u{259E}',
    // 0xC8
    '\u{258A}',
    '\u{1FB87}',
    '\u{1FB8A}',
    '\u{1FB99}',
    '\u{1FB98}',
    '\u{1FB6D}',
    '\u{1FB6F}',
    '\u{1FB6C}',
    // 0xD0
    '\u{1FB6E}',
    '\u{1FB9A}',
    '\u{1FB9B}',
    '\u{2598}',
    '\u{2597}',
    '\u{259D}',
    '\u{2596}',
    '\u{1FB96}',
    // 0xD8
    '\u{0394}',
    '\u{2021}',
    '\u{03C9}',
    '\u{2588}',
    '\u{2584}',
    '\u{258C}',
    '\u{2590}',
    '\u{2580}',
    // 0xE0
    '\u{03B1}',
    '\u{00DF}',
    '\u{0393}',
    '\u{03C0}',
    '\u{03A3}',
    '\u{03C3}',
    '\u{00B5}',
    '\u{03C4}',
    // 0xE8
    '\u{03A6}',
    '\u{0398}',
    '\u{03A9}',
    '\u{03B4}',
    '\u{221E}',
    '\u{2205}',
    '\u{2208}',
    '\u{2229}',
    // 0xF0
    '\u{2261}',
    '\u{00B1}',
    '\u{2265}',
    '\u{2264}',
    '\u{2320}',
    '\u{2321}',
    '\u{00F7}',
    '\u{2248}',
    // 0xF8 (0xFF is the cursor, rendered as a full block)
    '\u{00B0}',
    '\u{2219}',
    '\u{00B7}',
    '\u{221A}',
    '\u{207F}',
    '\u{00B2}',
    '\u{25A0}',
    '\u{2588}',
];

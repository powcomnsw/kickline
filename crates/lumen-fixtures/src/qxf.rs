//! Import QLC+ fixture definitions (`.qxf`).
//!
//! `.qxf` is the format of QLC+'s fixture library (Apache-2.0) and one of the
//! formats the Open Fixture Library exports, so this covers thousands of
//! fixtures. Each mode becomes a Lumen fixture type:
//!
//! - colour and dimmer channels are driven from the image;
//! - per-pixel "heads" become pixels laid out across the fixture;
//! - everything else (shutter, strobe, macros, pan/tilt...) is held at a
//!   safe value: the channel's default, "shutter open", "no function", or
//!   centre for pan/tilt.

use crate::{ChannelFn, FixtureType, Slot};

#[derive(Clone, Debug, PartialEq)]
pub struct QxfChannel {
    pub name: String,
    pub func: ChannelFn,
    pub fine: bool,
    /// QLC+ group / preset, for display ("Shutter", "PositionPan"...).
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QxfMode {
    pub name: String,
    /// Channel names in DMX order.
    pub channels: Vec<String>,
    /// Each head = indices into `channels`.
    pub heads: Vec<Vec<usize>>,
    pub layout: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QxfFixture {
    pub manufacturer: String,
    pub model: String,
    pub kind: String,
    pub channels: Vec<QxfChannel>,
    pub modes: Vec<QxfMode>,
    pub layout: Option<(usize, usize)>,
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(name))
}

fn children<'a, 'i: 'a>(n: roxmltree::Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> + 'a {
    n.children().filter(move |c| c.has_tag_name(name))
}

fn text(n: Option<roxmltree::Node>) -> String {
    n.and_then(|n| n.text()).unwrap_or("").trim().to_string()
}

fn layout_of(physical: Option<roxmltree::Node>) -> Option<(usize, usize)> {
    let l = child(physical?, "Layout")?;
    let w = l.attribute("Width")?.parse().ok()?;
    let h = l.attribute("Height")?.parse().ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

fn colour_fn(name: &str) -> Option<ChannelFn> {
    Some(match name {
        "Red" => ChannelFn::Red,
        "Green" => ChannelFn::Green,
        "Blue" => ChannelFn::Blue,
        "White" => ChannelFn::White,
        "Amber" => ChannelFn::Amber,
        "Cyan" => ChannelFn::Cyan,
        "Magenta" => ChannelFn::Magenta,
        "Yellow" => ChannelFn::Yellow,
        "UV" => ChannelFn::Uv,
        _ => return None,
    })
}

/// (function, fine) for a channel `Preset`, if it's one Lumen drives.
fn preset_fn(p: &str) -> Option<(ChannelFn, bool)> {
    let (base, fine) = match p.strip_suffix("Fine") {
        Some(b) => (b, true),
        None => (p, false),
    };
    let f = match base {
        "IntensityMasterDimmer" | "IntensityDimmer" => ChannelFn::Dimmer,
        _ => colour_fn(base.strip_prefix("Intensity")?)?,
    };
    Some((f, fine))
}

/// A safe constant for a channel Lumen doesn't drive.
fn hold_value(ch: roxmltree::Node, preset: Option<&str>) -> u8 {
    if let Some(d) = ch.attribute("Default").and_then(|d| d.parse::<u8>().ok()) {
        return d;
    }
    // Pan/tilt coarse: centre, so moving heads point somewhere sensible.
    if matches!(preset, Some("PositionPan" | "PositionTilt")) {
        return 128;
    }
    let caps: Vec<_> = children(ch, "Capability").collect();
    let min = |c: &roxmltree::Node| c.attribute("Min").and_then(|m| m.parse::<u8>().ok()).unwrap_or(0);
    // A shutter must be open or the light stays dark.
    if let Some(c) = caps.iter().find(|c| c.attribute("Preset") == Some("ShutterOpen")) {
        return min(c);
    }
    let group = text(child(ch, "Group"));
    if group == "Shutter" {
        if let Some(c) = caps.iter().find(|c| {
            let t = c.text().unwrap_or("").to_ascii_lowercase();
            t.contains("open") || t.contains("no strobe") || t.contains("no function")
        }) {
            return min(c);
        }
    }
    if let Some(c) = caps.iter().find(|c| {
        c.attribute("Preset") == Some("NoFunction") || c.text().unwrap_or("").to_ascii_lowercase().contains("no function")
    }) {
        return min(c);
    }
    0
}

fn parse_channel(ch: roxmltree::Node) -> QxfChannel {
    let name = ch.attribute("Name").unwrap_or("").to_string();
    let preset = ch.attribute("Preset");
    if let Some((func, fine)) = preset.and_then(preset_fn) {
        return QxfChannel { name, func, fine, kind: preset.unwrap_or("").to_string() };
    }
    let group = child(ch, "Group");
    let gname = text(group);
    let fine = group.and_then(|g| g.attribute("Byte")) == Some("1")
        || preset.is_some_and(|p| p.ends_with("Fine"));
    if gname == "Intensity" && preset.is_none() {
        let func = colour_fn(&text(child(ch, "Colour"))).unwrap_or(ChannelFn::Dimmer);
        return QxfChannel { name, func, fine, kind: "Intensity".into() };
    }
    let kind = preset.map(str::to_string).unwrap_or(gname);
    let v = if fine { 0 } else { hold_value(ch, preset) };
    QxfChannel { name, func: ChannelFn::Fixed(v), fine, kind }
}

pub fn parse(xml: &str) -> Result<QxfFixture, String> {
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() },
    )
    .map_err(|e| format!("not a valid fixture file: {e}"))?;
    let root = doc.root_element();
    if !root.has_tag_name("FixtureDefinition") {
        return Err("not a QLC+ fixture definition (.qxf)".into());
    }
    let channels: Vec<QxfChannel> = children(root, "Channel").map(parse_channel).collect();
    let modes: Vec<QxfMode> = children(root, "Mode")
        .map(|m| {
            let mut numbered: Vec<(usize, String)> = children(m, "Channel")
                .filter_map(|c| Some((c.attribute("Number")?.parse().ok()?, c.text().unwrap_or("").trim().to_string())))
                .collect();
            numbered.sort_by_key(|(n, _)| *n);
            let heads = children(m, "Head")
                .map(|h| children(h, "Channel").filter_map(|c| c.text()?.trim().parse().ok()).collect())
                .collect();
            QxfMode {
                name: m.attribute("Name").unwrap_or("").to_string(),
                channels: numbered.into_iter().map(|(_, n)| n).collect(),
                heads,
                layout: layout_of(child(m, "Physical")),
            }
        })
        .collect();
    if modes.is_empty() {
        return Err("fixture has no modes".into());
    }
    Ok(QxfFixture {
        manufacturer: text(child(root, "Manufacturer")),
        model: text(child(root, "Model")),
        kind: text(child(root, "Type")),
        channels,
        modes,
        layout: layout_of(child(root, "Physical")),
    })
}

impl QxfFixture {
    pub fn title(&self) -> String {
        format!("{} {}", self.manufacturer, self.model)
    }

    pub fn channel(&self, name: &str) -> Option<&QxfChannel> {
        self.channels.iter().find(|c| c.name == name)
    }

    /// Turn one mode into a Lumen fixture type.
    pub fn to_type(&self, mode: usize, source: &str) -> Result<FixtureType, String> {
        let m = self.modes.get(mode).ok_or("no such mode")?;
        let head_of = |i: usize| m.heads.iter().position(|h| h.contains(&i));
        let single = m.heads.len() <= 1;
        let slots: Vec<Slot> = m
            .channels
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let ch = self.channel(name);
                let func = ch.map(|c| c.func).unwrap_or(ChannelFn::Fixed(0));
                let drives_light = !matches!(func, ChannelFn::Fixed(_));
                // Single-head fixtures: every colour/dimmer channel belongs to pixel 0.
                let pixel = if single { drives_light.then_some(0) } else { head_of(i) };
                Slot { name: name.clone(), func, pixel, fine: ch.is_some_and(|c| c.fine) }
            })
            .collect();
        if slots.is_empty() {
            return Err(format!("mode \"{}\" has no channels", m.name));
        }
        let heads = m.heads.len().max(1);
        let (columns, rows) = match m.layout.or(self.layout) {
            Some((w, h)) if w * h == heads => (w, h),
            _ => (heads, 1),
        };
        Ok(FixtureType {
            name: format!("{} ({})", self.title(), m.name),
            columns,
            rows,
            pixel: Vec::new(),
            header: Vec::new(),
            slots: Some(slots),
            source: Some(source.to_string()),
        })
    }

    /// How many of a mode's channels Lumen drives from the image.
    pub fn driven_channels(&self, mode: usize) -> usize {
        self.modes.get(mode).map_or(0, |m| {
            m.channels
                .iter()
                .filter(|n| self.channel(n).is_some_and(|c| !matches!(c.func, ChannelFn::Fixed(_))))
                .count()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdvancedOutput, Fixture, Lumiverse};
    use lumen_core::{Frame, Rgba};

    const SLIMPAR: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE FixtureDefinition>
<FixtureDefinition xmlns="http://www.qlcplus.org/FixtureDefinition">
 <Manufacturer>Chauvet</Manufacturer>
 <Model>SlimPAR 56</Model>
 <Type>Color Changer</Type>
 <Channel Name="Red" Preset="IntensityRed"/>
 <Channel Name="Green" Preset="IntensityGreen"/>
 <Channel Name="Blue" Preset="IntensityBlue"/>
 <Channel Name="Color Macros"><Group Byte="0">Colour</Group>
  <Capability Min="0" Max="15">No function</Capability><Capability Min="16" Max="255">Color macros</Capability></Channel>
 <Channel Name="Strobe"><Group Byte="0">Shutter</Group>
  <Capability Min="0" Max="15" Preset="ShutterOpen">No function</Capability>
  <Capability Min="16" Max="255" Preset="StrobeSlowToFast">Slow ~ fast</Capability></Channel>
 <Channel Name="Dimmer" Preset="IntensityDimmer"/>
 <Mode Name="6-Ch">
  <Channel Number="0">Red</Channel><Channel Number="1">Green</Channel><Channel Number="2">Blue</Channel>
  <Channel Number="3">Color Macros</Channel><Channel Number="4">Strobe</Channel><Channel Number="5">Dimmer</Channel>
 </Mode>
 <Mode Name="3-Ch">
  <Channel Number="0">Red</Channel><Channel Number="1">Green</Channel><Channel Number="2">Blue</Channel>
 </Mode>
</FixtureDefinition>"#;

    /// Old-style groups, a shutter whose "open" is high, a master dimmer and 2 RGB heads.
    const BAR: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<FixtureDefinition xmlns="http://www.qlcplus.org/FixtureDefinition">
 <Manufacturer>Test</Manufacturer><Model>Bar 2</Model><Type>LED Bar (Pixels)</Type>
 <Channel Name="Master" Preset="IntensityMasterDimmer"/>
 <Channel Name="Shutter"><Group Byte="0">Shutter</Group>
  <Capability Min="0" Max="15" Preset="ShutterClose">Closed</Capability>
  <Capability Min="16" Max="239">Strobe</Capability>
  <Capability Min="240" Max="255">Open</Capability></Channel>
 <Channel Name="Pan" Preset="PositionPan"/>
 <Channel Name="R1"><Group Byte="0">Intensity</Group><Colour>Red</Colour></Channel>
 <Channel Name="G1"><Group Byte="0">Intensity</Group><Colour>Green</Colour></Channel>
 <Channel Name="B1"><Group Byte="0">Intensity</Group><Colour>Blue</Colour></Channel>
 <Channel Name="R2" Preset="IntensityRed"/>
 <Channel Name="G2" Preset="IntensityGreen"/>
 <Channel Name="B2" Preset="IntensityBlue"/>
 <Channel Name="Dim fine" Preset="IntensityMasterDimmerFine"/>
 <Mode Name="10ch">
  <Channel Number="0">Master</Channel><Channel Number="1">Shutter</Channel><Channel Number="2">Pan</Channel>
  <Channel Number="3">R1</Channel><Channel Number="4">G1</Channel><Channel Number="5">B1</Channel>
  <Channel Number="6">R2</Channel><Channel Number="7">G2</Channel><Channel Number="8">B2</Channel>
  <Channel Number="9">Dim fine</Channel>
  <Head><Channel>3</Channel><Channel>4</Channel><Channel>5</Channel></Head>
  <Head><Channel>6</Channel><Channel>7</Channel><Channel>8</Channel></Head>
 </Mode>
 <Physical><Layout Width="2" Height="1"/></Physical>
</FixtureDefinition>"#;

    fn render(t: FixtureType, frame: &Frame) -> Vec<u8> {
        let n = t.footprint();
        let o = AdvancedOutput {
            aspect: frame.width as f32 / frame.height as f32,
            lumiverses: vec![Lumiverse::new("L", 0)],
            types: vec![t],
            fixtures: vec![Fixture {
                name: "f".into(),
                type_id: 0,
                lumiverse: 0,
                address: 1,
                x: 0.5,
                y: 0.5,
                w: 1.0,
                h: 1.0,
                rotation: 0.0,
                snake: false,
                reverse: false,
            }],
        };
        o.render_dmx(frame)[&(0, 0)][..n].to_vec()
    }

    #[test]
    fn slimpar_modes_and_values() {
        let f = parse(SLIMPAR).unwrap();
        assert_eq!(f.title(), "Chauvet SlimPAR 56");
        assert_eq!(f.modes.len(), 2);
        assert_eq!(f.driven_channels(0), 4);
        let t = f.to_type(0, "test").unwrap();
        assert_eq!(t.footprint(), 6);
        assert_eq!(t.pixel_count(), 1);
        // half-bright orange: dimmer carries brightness, colour normalised; macro/strobe held open
        let dmx = render(t, &Frame::new(2, 2, Rgba::rgb(0.5, 0.25, 0.0)));
        assert_eq!(dmx, vec![255, 128, 0, 0, 0, 128]);
        let t3 = f.to_type(1, "test").unwrap();
        assert_eq!(render(t3, &Frame::new(1, 1, Rgba::rgb(0.5, 0.25, 0.0))), vec![128, 64, 0]);
    }

    #[test]
    fn pixel_bar_heads_shutter_and_master() {
        let f = parse(BAR).unwrap();
        let t = f.to_type(0, "test").unwrap();
        assert_eq!((t.columns, t.rows), (2, 1));
        // left half red, right half blue
        let mut fr = Frame::black(2, 1);
        fr.pixels[0] = Rgba::rgb(1.0, 0.0, 0.0);
        fr.pixels[1] = Rgba::rgb(0.0, 0.0, 1.0);
        let dmx = render(t, &fr);
        // master open (pixels carry colour), shutter open at 240 (found by "Open" text), pan centred,
        // head 1 red, head 2 blue, master fine = low byte of full = 255
        assert_eq!(dmx, vec![255, 240, 128, 255, 0, 0, 0, 0, 255, 255]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("hello").is_err());
        assert!(parse("<Workspace/>").is_err());
    }
}

//! Parser for PMDCollab AnimData.xml files.
//!
//! Each creature in the PMDCollab sprite repository has an AnimData.xml that
//! describes every available animation: frame dimensions and per-frame durations.
//! We parse this to know how to cut up sprite sheets and time animations.

use std::collections::HashMap;

/// Information about a single animation (e.g. "Idle", "Sleep", "Eat").
#[derive(Debug, Clone)]
pub struct AnimInfo {
    /// Width of each frame in pixels
    pub frame_width: u32,
    /// Height of each frame in pixels
    pub frame_height: u32,
    /// Duration of each frame in game ticks (1 tick ≈ 50ms)
    pub durations: Vec<u32>,
}

impl AnimInfo {
    /// Total number of frames in this animation.
    pub fn frame_count(&self) -> usize {
        self.durations.len()
    }

    /// Total duration of one full animation cycle in milliseconds.
    /// Each tick is ~50ms.
    #[cfg(test)]
    pub fn total_duration_ms(&self) -> u64 {
        self.durations.iter().map(|&d| d as u64 * 50).sum()
    }
}

/// Parse an AnimData.xml string into a map of animation name → AnimInfo.
///
/// The XML format looks like:
/// ```xml
/// <AnimData>
///   <Anims>
///     <Anim>
///       <Name>Idle</Name>
///       <FrameWidth>40</FrameWidth>
///       <FrameHeight>56</FrameHeight>
///       <Durations>
///         <Duration>40</Duration>
///         <Duration>2</Duration>
///       </Durations>
///     </Anim>
///     ...
///   </Anims>
/// </AnimData>
/// ```
///
/// We use simple string parsing since the XML is very regular and we don't
/// want to pull in a full XML library just for this.
pub fn parse_anim_data(xml: &str) -> HashMap<String, AnimInfo> {
    let mut result = HashMap::new();

    for anim_block in xml.split("<Anim>").skip(1) {
        let block = match anim_block.split("</Anim>").next() {
            Some(b) => b,
            None => continue,
        };

        let name = match extract_tag_value(block, "Name") {
            Some(n) => n,
            None => continue,
        };

        let frame_width = match extract_tag_value(block, "FrameWidth") {
            Some(v) => match v.parse::<u32>() {
                Ok(n) => n,
                Err(_) => continue,
            },
            None => continue,
        };

        let frame_height = match extract_tag_value(block, "FrameHeight") {
            Some(v) => match v.parse::<u32>() {
                Ok(n) => n,
                Err(_) => continue,
            },
            None => continue,
        };

        let durations = extract_durations(block);
        if durations.is_empty() {
            continue;
        }

        result.insert(
            name,
            AnimInfo {
                frame_width,
                frame_height,
                durations,
            },
        );
    }

    result
}

/// Extract the text content of a simple XML tag like `<Name>Idle</Name>`.
fn extract_tag_value(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");

    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&close)? + start;

    Some(block[start..end].trim().to_string())
}

/// Extract all `<Duration>N</Duration>` values from a block.
fn extract_durations(block: &str) -> Vec<u32> {
    let mut durations = Vec::new();

    for part in block.split("<Duration>").skip(1) {
        if let Some(end) = part.find("</Duration>")
            && let Ok(val) = part[..end].trim().parse::<u32>()
        {
            durations.push(val);
        }
    }

    durations
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sample AnimData.xml content for testing.
    const SAMPLE_XML: &str = r#"<?xml version="1.0" ?>
<AnimData>
    <ShadowSize>1</ShadowSize>
    <Anims>
        <Anim>
            <Name>Idle</Name>
            <Index>7</Index>
            <FrameWidth>40</FrameWidth>
            <FrameHeight>56</FrameHeight>
            <Durations>
                <Duration>40</Duration>
                <Duration>2</Duration>
                <Duration>3</Duration>
            </Durations>
        </Anim>
        <Anim>
            <Name>Sleep</Name>
            <Index>5</Index>
            <FrameWidth>32</FrameWidth>
            <FrameHeight>40</FrameHeight>
            <Durations>
                <Duration>30</Duration>
                <Duration>35</Duration>
            </Durations>
        </Anim>
        <Anim>
            <Name>Eat</Name>
            <Index>15</Index>
            <FrameWidth>24</FrameWidth>
            <FrameHeight>48</FrameHeight>
            <Durations>
                <Duration>6</Duration>
                <Duration>8</Duration>
                <Duration>6</Duration>
                <Duration>8</Duration>
            </Durations>
        </Anim>
    </Anims>
</AnimData>"#;

    #[test]
    fn test_parse_finds_all_animations() {
        let anims = parse_anim_data(SAMPLE_XML);
        assert_eq!(anims.len(), 3);
        assert!(anims.contains_key("Idle"));
        assert!(anims.contains_key("Sleep"));
        assert!(anims.contains_key("Eat"));
    }

    #[test]
    fn test_parse_idle_details() {
        let anims = parse_anim_data(SAMPLE_XML);
        let idle = &anims["Idle"];
        assert_eq!(idle.frame_width, 40);
        assert_eq!(idle.frame_height, 56);
        assert_eq!(idle.durations, vec![40, 2, 3]);
        assert_eq!(idle.frame_count(), 3);
    }

    #[test]
    fn test_parse_sleep_details() {
        let anims = parse_anim_data(SAMPLE_XML);
        let sleep = &anims["Sleep"];
        assert_eq!(sleep.frame_width, 32);
        assert_eq!(sleep.frame_height, 40);
        assert_eq!(sleep.durations, vec![30, 35]);
    }

    #[test]
    fn test_parse_eat_details() {
        let anims = parse_anim_data(SAMPLE_XML);
        let eat = &anims["Eat"];
        assert_eq!(eat.frame_width, 24);
        assert_eq!(eat.frame_height, 48);
        assert_eq!(eat.durations, vec![6, 8, 6, 8]);
        assert_eq!(eat.frame_count(), 4);
    }

    #[test]
    fn test_total_duration_ms() {
        let anims = parse_anim_data(SAMPLE_XML);
        let idle = &anims["Idle"];
        // (40 + 2 + 3) * 50 = 2250ms
        assert_eq!(idle.total_duration_ms(), 2250);
    }

    #[test]
    fn test_empty_xml() {
        let anims = parse_anim_data("");
        assert!(anims.is_empty());
    }

    #[test]
    fn test_malformed_xml_skipped() {
        let xml = r#"<Anim><Name>Bad</Name></Anim>"#;
        let anims = parse_anim_data(xml);
        // Missing FrameWidth/FrameHeight/Durations, should be skipped
        assert!(anims.is_empty());
    }
}

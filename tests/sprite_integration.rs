//! Integration test: download Pikachu sprites and verify frame counts.
//! This test requires network access.

use poclimon::anim_data;
use poclimon::sprite_sheet;

#[test]
fn test_pikachu_sprite_download_and_frame_extraction() {
    // Download AnimData.xml for Pikachu (ID 25)
    let url = "https://raw.githubusercontent.com/PMDCollab/SpriteCollab/master/sprite/0025/AnimData.xml";
    let output = std::process::Command::new("curl")
        .args(["-sL", url])
        .output()
        .expect("curl should work");

    let xml = String::from_utf8(output.stdout).expect("valid UTF-8");
    assert!(xml.contains("<AnimData>"), "Should be valid AnimData XML");

    let anims = anim_data::parse_anim_data(&xml);

    // Pikachu should have Idle, Sleep, and Eat animations
    assert!(anims.contains_key("Idle"), "Should have Idle animation");
    assert!(anims.contains_key("Sleep"), "Should have Sleep animation");
    assert!(anims.contains_key("Eat"), "Should have Eat animation");

    // Verify Idle has expected properties
    let idle = &anims["Idle"];
    assert_eq!(idle.frame_width, 40);
    assert_eq!(idle.frame_height, 56);
    assert_eq!(idle.frame_count(), 6); // Pikachu Idle has 6 frames

    // Download the Idle sprite sheet and extract frames
    let sheet_url =
        "https://raw.githubusercontent.com/PMDCollab/SpriteCollab/master/sprite/0025/Idle-Anim.png";
    let tmp_dir = std::env::temp_dir().join("poclimon_test");
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let sheet_path = tmp_dir.join("pikachu_idle.png");

    let dl = std::process::Command::new("curl")
        .args(["-sL", "-o"])
        .arg(sheet_path.as_os_str())
        .arg(sheet_url)
        .output()
        .expect("curl should work");
    assert!(dl.status.success(), "Sprite sheet download should succeed");

    let sheet = image::open(&sheet_path).expect("Should open sprite sheet PNG");
    let frames = sprite_sheet::extract_frames(&sheet, idle);

    // Frame count from extraction should match AnimData duration count
    assert_eq!(
        frames.len(),
        idle.frame_count(),
        "Extracted frame count should match AnimData"
    );

    // Each frame should have the right dimensions
    for (i, frame) in frames.iter().enumerate() {
        let (w, h) = frame.dimensions();
        assert_eq!(w, idle.frame_width, "Frame {} width mismatch", i);
        assert_eq!(h, idle.frame_height, "Frame {} height mismatch", i);
    }

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

use image::GenericImageView;

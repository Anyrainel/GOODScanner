#![cfg(feature = "capture")]

use hsr_scanner::{
    build_export, load_embedded_gilore_reference,
    packet_capture::{
        proto::{
            Avatar::Avatar, AvatarPathData::AvatarPathData,
            AvatarPathSkillTree::AvatarPathSkillTree, Equipment::Equipment, Relic::Relic,
            RelicAffix::RelicAffix,
        },
        HsrPacketDecoder,
    },
    ValidatedObservationSnapshot,
};
use protobuf::{CodedOutputStream, Message};

fn group(tag: u32, messages: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut output = CodedOutputStream::vec(&mut bytes);
    for message in messages {
        output.write_bytes(tag, message).unwrap();
    }
    output.flush().unwrap();
    drop(output);
    bytes
}

fn characters(tag: u32, path_tag: u32) -> Vec<u8> {
    character_packet(tag, path_tag, true)
}

fn character_packet(tag: u32, path_tag: u32, get_all: bool) -> Vec<u8> {
    let base = Avatar {
        base_avatar_id: 1001,
        level: 80,
        promotion: 6,
        first_met_time_stamp: 1_700_000_000,
        ..Default::default()
    };
    let path = AvatarPathData {
        avatar_id: 1001,
        rank: 2,
        avatar_path_skill_tree: vec![AvatarPathSkillTree {
            point_id: 1001001,
            level: 6,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut bytes = group(tag, &[base.write_to_bytes().unwrap()]);
    bytes.extend(group(path_tag, &[path.write_to_bytes().unwrap()]));
    let mut out = CodedOutputStream::vec(&mut bytes);
    out.write_bool(700, get_all).unwrap();
    out.flush().unwrap();
    drop(out);
    bytes
}

#[test]
fn character_sync_is_not_a_complete_roster() {
    let mut decoder =
        HsrPacketDecoder::new(load_embedded_gilore_reference().unwrap(), false).unwrap();
    decoder
        .receive_command(&character_packet(13, 77, false))
        .unwrap();
    assert!(!decoder.state().has_characters);
    decoder.receive_command(&characters(13, 77)).unwrap();
    assert!(decoder.state().has_characters);
}

#[test]
fn trailblazer_path_uses_base_progression() {
    let mut decoder =
        HsrPacketDecoder::new(load_embedded_gilore_reference().unwrap(), false).unwrap();
    let base = Avatar {
        base_avatar_id: 8001,
        level: 70,
        promotion: 5,
        first_met_time_stamp: 1_700_000_000,
        ..Default::default()
    };
    let path = AvatarPathData {
        avatar_id: 8005,
        rank: 4,
        avatar_path_skill_tree: vec![AvatarPathSkillTree {
            point_id: 8005001,
            level: 6,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut bytes = group(31, &[base.write_to_bytes().unwrap()]);
    bytes.extend(group(47, &[path.write_to_bytes().unwrap()]));
    let mut out = CodedOutputStream::vec(&mut bytes);
    out.write_bool(800, true).unwrap();
    out.flush().unwrap();
    drop(out);
    decoder.receive_command(&bytes).unwrap();
    decoder.receive_command(&bag(6, 7)).unwrap();
    let character = &decoder.state().inventory.as_ref().unwrap().characters[0];
    assert_eq!(
        (
            character.character_id,
            character.level,
            character.ascension,
            character.eidolon
        ),
        (8005, 70, 5, 4)
    );
}

fn bag(tag: u32, relic_tag: u32) -> Vec<u8> {
    let cone = Equipment {
        tid: 23005,
        unique_id: 77,
        level: 80,
        promotion: 6,
        rank: 3,
        equip_avatar_id: 1001,
        is_protected: true,
        ..Default::default()
    };
    let relic = Relic {
        tid: 61011,
        unique_id: 88,
        level: 15,
        main_affix_id: 1,
        equip_avatar_id: 1001,
        is_protected: true,
        sub_affix_list: vec![RelicAffix {
            affix_id: 7,
            cnt: 2,
            step: 2,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut bytes = group(tag, &[cone.write_to_bytes().unwrap()]);
    bytes.extend(group(relic_tag, &[relic.write_to_bytes().unwrap()]));
    bytes
}

#[test]
fn rotated_outer_fields_and_nested_wrappers_export_inventory() {
    let references = load_embedded_gilore_reference().unwrap();
    for (a, b) in [(1, 2), (777, 991), (31, 42)] {
        let mut decoder = HsrPacketDecoder::new(references.clone(), false).unwrap();
        decoder
            .receive_command(&group(63, &[characters(a, b)]))
            .unwrap();
        assert!(decoder.state().has_characters);
        assert!(!decoder.state().complete);
        decoder.receive_command(&group(55, &[bag(b, a)])).unwrap();
        let state = decoder.state();
        assert!(state.complete);
        assert_eq!(
            (
                state.character_count,
                state.light_cone_count,
                state.relic_count
            ),
            (1, 1, 1)
        );
        let export = build_export(
            ValidatedObservationSnapshot::from_packet_capture(state.inventory.clone().unwrap())
                .unwrap(),
            &references,
        )
        .unwrap();
        assert_eq!(export.characters[0].eidolon, 2);
        assert_eq!(export.light_cones[0].superimposition, 3);
        assert_eq!(export.relics[0].gear.level, 15);
        let json = serde_json::to_string(&export).unwrap();
        assert!(!json.contains("unique_id"));
        assert!(!json.contains("first_met"));
    }
}

#[test]
fn achievements_are_optional_but_cannot_complete_inventory_capture_alone() {
    let refs = load_embedded_gilore_reference().unwrap();
    let mut decoder = HsrPacketDecoder::new(refs.clone(), true).unwrap();
    let records = refs
        .achievement_ids()
        .take(6)
        .map(|id| {
            let mut bytes = Vec::new();
            let mut out = CodedOutputStream::vec(&mut bytes);
            out.write_uint32(901, id).unwrap();
            out.write_uint32(333, 3).unwrap();
            out.write_uint32(221, 1_700_000_000).unwrap();
            // Current Quest records may have additional packed counters.
            out.write_bytes(19, &[1, 2, 3]).unwrap();
            out.flush().unwrap();
            drop(out);
            bytes
        })
        .collect::<Vec<_>>();
    decoder
        .receive_command(&group(67, &[group(456, &records)]))
        .unwrap();
    assert!(decoder.state().has_achievements);
    assert_eq!(decoder.state().achievement_count, 6);
    assert!(!decoder.state().complete);
    decoder.receive_command(&bag(6, 7)).unwrap();
    assert!(!decoder.state().complete);
    decoder.receive_command(&characters(22, 23)).unwrap();
    assert!(decoder.state().complete);
}

#[test]
fn malformed_and_ambiguous_records_do_not_claim_complete_data() {
    let refs = load_embedded_gilore_reference().unwrap();
    let mut decoder = HsrPacketDecoder::new(refs, false).unwrap();
    for bytes in [&[0xff; 8][..], &[0x12, 0xff], &[]] {
        decoder.receive_command(bytes).unwrap();
        assert!(!decoder.state().complete);
    }
    // Two independently valid base collections are ambiguous.
    let mut ambiguous = characters(6, 7);
    let base = Avatar {
        base_avatar_id: 1001,
        level: 80,
        promotion: 6,
        first_met_time_stamp: 1_700_000_000,
        ..Default::default()
    };
    ambiguous.extend(group(8, &[base.write_to_bytes().unwrap()]));
    decoder.receive_command(&ambiguous).unwrap();
    assert!(!decoder.state().has_characters);
}

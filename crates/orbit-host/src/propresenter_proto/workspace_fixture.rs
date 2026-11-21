use super::*;
use std::{fs, path::PathBuf};
const IMAGE: &str = "orbit-int12-image.png";
const VIDEO: &str = "orbit-int12-video.mp4";
// Only the two owned absolute media strings are relocated. All source objects,
// styling, flags, IDs and unknown protobuf fields remain in the authentic fixture.
pub(crate) fn relocate(bytes: &[u8], assets: &Path) -> Vec<u8> {
    let Ok(fields) = decode_message(bytes) else {
        return bytes.to_vec();
    };
    let mut output = Vec::new();
    for field in fields {
        match field.value {
            WireValue::Varint(value) => put_varint(field.number, value, &mut output),
            WireValue::Fixed64(value) => {
                encode_varint((field.number as u64) << 3 | 1, &mut output);
                output.extend(value.to_le_bytes());
            }
            WireValue::Fixed32(value) => {
                encode_varint((field.number as u64) << 3 | 5, &mut output);
                output.extend(value.to_le_bytes());
            }
            WireValue::Bytes(value) => {
                let rewritten = [IMAGE, VIDEO]
                    .into_iter()
                    .find(|name| {
                        value
                            == format!(
                                "C:\\Users\\Demo\\Documents\\ProPresenter\\Media\\Assets\\{name}"
                            )
                            .as_bytes()
                    })
                    .map(|name| assets.join(name).to_string_lossy().as_bytes().to_vec())
                    .unwrap_or_else(|| relocate(&value, assets));
                put_bytes(field.number, &rewritten, &mut output);
            }
        }
    }
    output
}

fn m(number: u32, bytes: Vec<u8>) -> Vec<u8> {
    let mut result = Vec::new();
    put_message(number, bytes, &mut result);
    result
}
fn s(number: u32, text: &str) -> Vec<u8> {
    let mut result = Vec::new();
    put_string(number, text, &mut result);
    result
}
fn u(number: u32, text: &str) -> Vec<u8> {
    let mut result = Vec::new();
    put_uuid(number, text, &mut result);
    result
}
fn v(number: u32, value: u64) -> Vec<u8> {
    let mut result = Vec::new();
    put_varint(number, value, &mut result);
    result
}
fn doc(fields: &[Vec<u8>]) -> Vec<u8> {
    fields.concat()
}

// Collection wrappers follow the vendored schemas. The slide subtree is authentic,
// saved by ProPresenter 21.4.2; only its owned media paths are relocated.
pub(crate) fn add_configuration_families(source: &Path, assets: &Path) {
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/propresenter");
    fs::create_dir_all(assets).unwrap();
    for name in [IMAGE, VIDEO] {
        fs::copy(fixtures.join(name), assets.join(name)).unwrap();
    }
    let presentation = relocate(
        &fs::read(fixtures.join("authentic-layered-21.pro")).unwrap(),
        assets,
    );
    fs::write(
        source.join("Libraries/Orbit INT-15 Other/Authentic.pro"),
        &presentation,
    )
    .unwrap();
    let fields = decode_message(&presentation).unwrap();
    let cue = nested_fields(&fields, 13).next().unwrap();
    let slide = nested_fields(&cue, 10)
        .find_map(|action| nested_field(&action, 23).and_then(|slide| nested_field(&slide, 2)))
        .unwrap();
    let base = bytes_field(&slide, 1).unwrap().to_vec();
    let config = source.join("Configuration");
    let prop = "8f000000-0000-4000-8000-000000000004";
    let prop_action = doc(&[v(9, 15), m(23, m(3, m(1, base.clone())))]);
    let prop_cue = doc(&[u(1, prop), s(2, "Authentic media prop"), m(10, prop_action)]);
    fs::write(config.join("Props"), m(2, prop_cue)).unwrap();
    fs::write(
        config.join("Stage"),
        m(
            2,
            doc(&[
                u(1, "8f000000-0000-4000-8000-000000000003"),
                s(2, "Authentic media stage"),
                m(3, base.clone()),
            ]),
        ),
    )
    .unwrap();
    let screen = "8f000000-0000-4000-8000-000000000011";
    let screen_look = doc(&[u(1, screen), v(7, 1)]);
    let layout = "8f000000-0000-4000-8000-000000000003";
    let workspace = doc(&[
        m(
            1,
            doc(&[s(1, "Imported stage"), v(2, 2), u(7, screen), v(9, 0)]),
        ),
        m(
            2,
            doc(&[
                u(1, "8f000000-0000-4000-8000-000000000012"),
                s(2, "Imported look"),
                m(3, screen_look),
            ]),
        ),
        m(4, doc(&[m(1, base), s(2, "Authentic media mask")])),
        m(
            5,
            doc(&[
                u(1, "8f000000-0000-4000-8000-000000000013"),
                s(2, "Imported camera"),
                m(3, s(2, "Test camera")),
            ]),
        ),
        m(6, doc(&[m(1, u(1, screen)), m(2, s(2, layout))])),
        m(
            11,
            doc(&[
                u(1, "8f000000-0000-4000-8000-000000000014"),
                s(2, "Imported audio"),
                m(5, m(1, Vec::new())),
            ]),
        ),
        m(
            9,
            m(
                2,
                doc(&[
                    u(1, "8f000000-0000-4000-8000-000000000015"),
                    s(2, "Imported recording"),
                    m(3, m(3, doc(&[v(1, 1), v(2, 1280), v(3, 720)]))),
                ]),
            ),
        ),
    ]);
    fs::write(config.join("Workspace"), workspace).unwrap();
    fs::write(
        config.join("KeyMappings"),
        m(2, doc(&[m(1, s(1, "F8")), m(105, u(1, prop))])),
    )
    .unwrap();
    fs::write(
        config.join("Communications"),
        m(
            1,
            doc(&[
                u(1, "8f000000-0000-4000-8000-000000000016"),
                s(2, "Imported TCP"),
                v(3, 4),
                m(8, doc(&[s(3, "127.0.0.1"), v(4, 5250)])),
            ]),
        ),
    )
    .unwrap();
    fs::create_dir(config.join("Planning Center")).unwrap();
    fs::write(
        config.join("Planning Center/Plan"),
        doc(&[
            v(1, 314),
            s(3, "Test series"),
            s(4, "Imported plan"),
            s(9, "plan-314"),
        ]),
    )
    .unwrap();
    let macro_id = "8f000000-0000-4000-8000-000000000007";
    fs::write(
        config.join("Calendar"),
        m(
            1,
            doc(&[
                u(1, "8f000000-0000-4000-8000-000000000017"),
                s(2, "Imported event"),
                m(4, v(1, 1_900_000_000)),
                m(8, m(4, m(1, u(1, macro_id)))),
            ]),
        ),
    )
    .unwrap();
    // A header action uses the same prop source UUID as the keyboard binding.
    let header = doc(&[
        u(1, "8f000000-0000-4000-8000-000000000018"),
        s(2, "Prop header"),
        m(3, m(2, doc(&[v(9, 6), m(28, m(3, u(1, prop)))]))),
    ]);
    let playlist = doc(&[
        u(1, "8f000000-0000-4000-8000-000000000019"),
        s(2, "Header actions"),
        v(3, 1),
        m(13, m(1, header)),
    ]);
    fs::write(source.join("Playlists/Header.proplaylist"), m(3, playlist)).unwrap();
}

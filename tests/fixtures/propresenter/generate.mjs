// Synthetic protobuf fixtures, built independently of Orbit's encoder.
// Field numbers follow vendor/ProPresenter7-Proto/Proto7.16.2.
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const join = (...parts) => Buffer.concat(parts);
function varint(number) {
  const bytes = [];
  do {
    bytes.push((number & 127) | (number > 127 ? 128 : 0));
    number = Math.floor(number / 128);
  } while (number);
  return Buffer.from(bytes);
}
const scalar = (field, value) => join(varint(field * 8), varint(value));
const message = (field, value) => join(varint(field * 8 + 2), varint(value.length), value);
const string = (field, value) => message(field, Buffer.from(value));
const uuid = (field, value) => message(field, string(1, value));
function double(field, value) {
  const bytes = Buffer.alloc(8);
  bytes.writeDoubleLE(value);
  return join(varint(field * 8 + 1), bytes);
}
const id = (number) => `00000000-0000-4000-8000-${String(number).padStart(12, '0')}`;
const size = (width, height) => join(double(1, width), double(2, height));
const bounds = (x, y, width, height) => join(message(1, size(x, y)), message(2, size(width, height)));
const application = (major = 7) => message(1, join(scalar(1, 2), scalar(3, 1), message(4, join(scalar(1, major), scalar(2, major === 7 ? 16 : 0), scalar(3, 2)))));
function text(number, name, rtf, x, y, width, height) {
  const font = join(string(1, 'Arial'), double(2, 42));
  const content = join(message(3, message(1, font)), string(5, rtf), scalar(6, 1));
  return message(1, join(message(1, join(uuid(1, id(number)), string(2, name), message(3, bounds(x, y, width, height)), message(13, content))), scalar(4, 2)));
}
function media(number, name, url, type, x, y, width, height) {
  const properties = type === 6 ? message(3, scalar(12, 1)) : Buffer.alloc(0);
  const asset = join(uuid(1, id(number)), message(2, string(1, url)), message(type, properties));
  return message(1, message(1, join(uuid(1, id(number)), string(2, name), message(3, bounds(x, y, width, height)), message(9, join(message(3, asset), scalar(4, 1))))));
}
function base(elements, width = 1920, height = 1080) {
  return join(...elements, scalar(4, 0), message(6, size(width, height)));
}
function cue(number, label, slide, extra = Buffer.alloc(0)) {
  const action = join(uuid(1, id(number + 100)), scalar(9, 11), message(23, message(2, message(1, slide))));
  return message(13, join(uuid(1, id(number)), string(2, label), message(10, action), scalar(12, 1), extra));
}
const primary = text(201, 'Lyrics', '{\\rtf1\\ansi Sing\\par Peace \\u233?}', 100, 120, 1600, 600);
const secondary = text(202, 'Copyright', '{\\rtf1\\ansi Copyright 2026}', 120, 900, 1500, 100);
const layered = join(application(), uuid(2, id(1)), string(3, 'Layered song'), string(7, '{\\rtf1\\ansi Operator notes}'), cue(11, 'Verse 1', base([primary, secondary])));
const emptyCue = message(13, join(uuid(1, id(12)), string(2, 'Pause'), scalar(12, 0)));
const emptyText = cue(13, 'Silent lyrics', base([text(203, 'Lyrics', '{\\rtf1\\ansi }', 100, 120, 1600, 600), secondary]));
const group = (number, name, cues) => message(12, join(message(1, join(uuid(1, id(number)), string(2, name))), ...cues.map((number) => uuid(2, id(number)))));
const arrangement = message(11, join(uuid(1, id(31)), string(2, 'Repeat verse'), uuid(3, id(21)), uuid(3, id(22)), uuid(3, id(21))));
const emptyGroups = join(application(), uuid(2, id(2)), string(3, 'Empty and grouped'), uuid(10, id(31)), arrangement, group(21, 'Verse 1', [11, 13]), group(22, 'Pause', [12]), cue(11, 'Verse 1', base([primary])), emptyCue, emptyText);
const logoUrl = 'file:///C:/Corpus/logo.png';
const videoUrl = 'file:///C:/Corpus/background.mp4';
const mediaAction = message(10, join(uuid(1, id(301)), scalar(9, 12), message(20, join(message(5, join(uuid(1, id(302)), message(2, string(1, videoUrl)), message(6, message(3, scalar(12, 1))))), scalar(10, 1)))));
const mediaPresentation = join(application(), uuid(2, id(3)), string(3, 'Media references'), cue(14, 'Media', base([primary, media(204, 'Logo', logoUrl, 5, 1700, 900, 160, 100), media(205, 'Background', videoUrl, 6, 0, 0, 1920, 1080)]), mediaAction));
const theme = join(application(), message(3, join(message(1, base([primary, secondary], 1280, 720)), string(2, 'Two text regions'))), message(3, join(message(1, base([], 1280, 720)), string(2, 'Blank'))));
const unknown = join(application(21), uuid(2, id(1)), string(3, 'Layered song'), string(7, '{\\rtf1\\ansi Operator notes}'), string(19000, 'Synthetic unknown extension'), cue(11, 'Verse 1', base([primary, secondary])));
const write = (file, bytes) => writeFileSync(fileURLToPath(new URL(file, import.meta.url)), bytes);
for (const [file, bytes] of Object.entries({ 'layered.pro': layered, 'empty-groups-arrangements.pro': emptyGroups, 'media.pro': mediaPresentation, 'Theme': theme, 'unknown-version.pro': unknown, 'truncated.pro': Buffer.from([0x80]) })) write(file, bytes);

const layeredExpected = {
  '/title': 'Layered song', '/sourceId': id(1), '/operatorNotes': 'Operator notes',
  '/slides/0/content': 'Sing\nPeace é', '/slides/0/label': 'Verse 1',
  '/slides/0/sourceCueId': id(11), '/slides/0/primaryTextLayerId': id(201),
  '/slides/0/layers/0/text': 'Sing\nPeace é', '/slides/0/layers/1/text': 'Copyright 2026',
  '/slides/0/layers/0/bounds': { x: 100, y: 120, width: 1600, height: 600 },
  '/slides/0/layers/1/bounds': { x: 120, y: 900, width: 1500, height: 100 },
  '/slides/0/layers/0/textStyle/fontFamily': 'Arial', '/slides/0/layers/0/textStyle/fontSize': 42,
};
const manifest = { provenance: 'synthetic', schema: 'vendored Proto7.16.2', cases: [
  { file: 'layered.pro', kind: 'presentation', slideCount: 1, expected: layeredExpected },
  { file: 'empty-groups-arrangements.pro', kind: 'presentation', slideCount: 3, expected: {
    '/slides/0/groupId': 'verse-1', '/slides/1/groupId': 'pause', '/slides/1/groupName': 'Pause',
    '/slides/1/content': '', '/slides/1/disabled': true, '/slides/1/sourceCueId': id(12),
    '/slides/2/content': '', '/slides/2/layers/0/text': '', '/slides/2/layers/1/text': 'Copyright 2026',
    '/groups/0/sourceId': id(21), '/groups/1/name': 'Pause', '/selectedArrangementId': id(31),
    '/arrangements/0/name': 'Repeat verse', '/arrangements/0/sectionIds': [id(21), id(22), id(21)],
  } },
  { file: 'media.pro', kind: 'presentation', slideCount: 1, expected: {
    '/slides/0/layers/1/type': 'Media', '/slides/0/layers/1/mediaUrl': logoUrl,
    '/slides/0/layers/2/type': 'Video', '/slides/0/layers/2/mediaUrl': videoUrl,
    '/slides/0/layers/2/mediaPlayback': 'Loop', '/slides/0/actions/1/type': 'media',
    '/slides/0/actions/1/targetId': videoUrl, '/slides/0/actions/1/details/mediaType': 'video',
    '/slides/0/actions/1/details/playback': 'PLAYBACK_BEHAVIOR_LOOP',
  } },
  { file: 'Theme', kind: 'template', slideCount: 2, expected: {
    '/slides/0/name': 'Two text regions', '/slides/0/width': 1280, '/slides/0/height': 720,
    '/slides/0/layers/0/text': 'Sing\nPeace é', '/slides/0/layers/1/text': 'Copyright 2026',
    '/slides/1/name': 'Blank', '/slides/1/content': '', '/slides/1/backgroundEnabled': false,
  } },
  { file: 'unknown-version.pro', kind: 'presentation', slideCount: 1, expected: layeredExpected },
] };
write('manifest.json', `${JSON.stringify(manifest, null, 2)}\n`);

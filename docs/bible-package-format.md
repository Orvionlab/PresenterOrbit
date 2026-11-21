# Bible translation package format

Presenter Orbit imports local Bible translations as a strict JSON document. A package is a UTF-8 JSON file with this shape:

```json
{
  "schema": "presenter-orbit.bible-package",
  "version": 1,
  "translation": {
    "id": "sample-en",
    "name": "Sample English Translation",
    "abbreviation": "SET",
    "language": "en",
    "sourceUrl": "https://example.org/translation",
    "license": {
      "name": "Public domain",
      "url": "https://example.org/license",
      "notice": "Optional attribution or license notice."
    }
  },
  "books": [
    {
      "id": "GEN",
      "name": "Genesis",
      "chapters": [
        {
          "number": 1,
          "verses": [
            { "number": 1, "text": "Sample verse text." },
            { "number": 2, "text": "Another sample verse." }
          ]
        }
      ]
    }
  ]
}
```

The top-level fields and every nested object are strict: unknown fields are rejected. `schema` and `version` must be exactly `presenter-orbit.bible-package` and `1`. Translation IDs use lowercase ASCII letters and digits, with optional interior hyphens. Both URLs must use HTTPS. The license `notice` is optional.

Each book ID is a three-character USFM-style identifier: uppercase ASCII letters, optionally preceded by a nonzero digit (`GEN`, `TOB`, and `1SA` are examples). Book IDs and names must be unique within their respective lists. Each book must contain chapters and each chapter must contain verses. Chapter and verse numbers are positive integers no greater than 200; verse text must be nonempty. Books, chapters, and verses may be partial or omit numbers. An explicitly numbered range crossing a missing verse returns an error. Whole chapters contain only their available verse numbers; the app never invents missing text. Parallel translations must contain the same requested verse numbers.

Packages may contain up to 200 books, 100,000 verses, and 20 MiB of JSON data. The app stores an installed copy in its application data and does not modify the source file.

## Built-in public-domain installer

**View → Bibles → Install World English Bible** downloads the Protestant World English Bible from [eBible.org](https://ebible.org/details.php?id=engwebp). The source identifies this translation as public domain. The installer reads the official `engwebp_vpl.txt` entry from its VPL ZIP, maps the publisher's book identifiers to USFM codes, and retains the source and license metadata. It does not bundle a translation marketplace or grant rights to other publishers' texts.

Installation requires a network connection; subsequent passage lookup, search, and slide generation run locally. An existing translation ID is never overwritten. Removing an installed package leaves the original import file and already-generated presentations intact.

## Browser and scripture presentations

Choose a translation, book and chapter range, or enter a reference such as `John 3:16-4:2`. Full installed book names and book IDs are accepted. Keyword search matches all entered terms, ignoring letter case, and returns pages of 100 verses. A result can load its verse or chapter.

Choose up to eight translations for a parallel passage. Their requested verse numbering must match; mismatched numbering produces an explicit error. Generation accepts 1–500 verses from a range spanning at most eleven chapters. The browser may display a larger passage, but generation rejects an oversized selection explicitly.

Select an existing theme and assign one distinct Text object to each translation. An optional additional Text object receives the reference; otherwise the reference accompanies the primary translation. Parallel layouts require enough separate Text objects in the theme. Verse numbers, translation labels, verse breaks and reference placement are configurable. Any package license notice remains attached to its text.

With verse breaks disabled, the generator packs whole verses into successive slides using the native renderer's wrapped font metrics and the smallest mapped text box. It reserves space for attribution and references. A verse that cannot fit alone produces an error before saving; enlarge the theme text objects or reduce their font size. Text is never silently truncated. The generated presentation is stored independently of the installed packages and can optionally be added to a regular playlist.

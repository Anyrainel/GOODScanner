# Shared Genshin reference

Scanner, capture and manager use
`https://ggartifact.com/good/genshin_scanner_data.json`, cached as
`data/genshin_scanner_data.json` with the successful fetch timestamp. The versioned
document contains both OCR mappings and the complete packet inventory catalog.
Normal loads reuse it for two hours and can fall back to a validated local cache
when updating fails. Forced refresh failures report an error and preserve the
last valid file. Loading the new file removes the obsolete separate cache and
metadata files without changing user settings or exports.

HoyoData generates the file after GenshinTools' OCR mappings are rebuilt. Both
producer and client validate the format, capture provenance, nonempty OCR mapping
collections, complete capture tables and artifact catalog. Clients no longer
download a historical artifact catalog to repair incomplete published data.

Achievements remain in `mapping_achievements.json`. Only an achievement scan loads
this catalog. The Scanner refresh button includes it when achievement scanning
is selected; Manager and Capture refresh only the shared inventory reference.

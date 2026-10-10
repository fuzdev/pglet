-- `$1::text::<enum>` — the spine binds an enum value as a text parameter and
-- casts it to the user enum type, then reads it back as text. Mirrors the
-- `Params text::enum` extended-protocol case in `test_binary`. The CAST path
-- resolves the enum name against the catalog (enums round-trip as their TEXT
-- label), so the bound `$1` is typed `text` by its inner cast and the projection
-- returns the label unchanged.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');

-- @param $1 text public
-- @test
SELECT ($1::text::cell_visibility)::text AS v;

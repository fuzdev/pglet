-- The fold reaches the **catalog**, not just resolution: an introspecting client sees
-- the downcased name. This is the discriminating shape for the whole fold rule — a fixture
-- that creates a name in one case and reads it in the same case passes under a folding
-- and a non-folding engine alike, because both sides move together; here only one side
-- does, since the `WHERE` and the projected values are literal text.
-- Both polarities in one query: `Bar` is stored `bar`, `"Baz"` is stored `Baz`.
-- @setup
CREATE TABLE CatFold (Bar INT, "Baz" INT);

-- @test
SELECT table_name::text AS table_name, column_name::text AS column_name
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'catfold'
ORDER BY ordinal_position;

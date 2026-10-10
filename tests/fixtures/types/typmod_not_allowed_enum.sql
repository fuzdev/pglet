-- An enum's `typmodin` is 0 (`typtype = 'e'`), so a modifier on one is the same
-- `42601` naming the enum. The refusal is keyed on the **base** name, which is what
-- lets the catalog be consulted at all — keying it on the whole `moodtm(3)` spelling
-- found no enum and answered about a type name nobody wrote.
-- @setup
CREATE TYPE moodtm AS ENUM ('ok', 'bad');
-- @expect-error: an enum takes no type modifier
-- @expect-message
-- @test
SELECT 'ok'::moodtm(3) AS never_reached;

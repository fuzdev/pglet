-- The duplicate is keyed on the **column**, not on the spelling: `"a"` and `a` are one
-- name once the parser's fold has run, so the quoted half is refused exactly as the bare
-- one is. 18.6 keys it on the resolved attribute number (a `Bitmapset`), which is the
-- same key.
-- @setup
CREATE TABLE dtcq (a INT, b INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtcq ("a", a) VALUES (1, 2);

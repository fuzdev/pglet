-- The accepting twin of `on_conflict_duplicate_set_target_over_a_zero_row_source_loud.sql`:
-- the same statement with a single assignment is `INSERT 0 0` on both engines and leaves
-- the table as it stood — and the table after it is what this fixture compares. It is
-- what makes that fixture's `42601` a statement about the repeat rather than about the
-- zero-row source.
-- @setup
CREATE TABLE ocdzk (a INT UNIQUE, b INT);
CREATE TABLE ocdzksrc (a INT, b INT);
INSERT INTO ocdzk VALUES (1, 1);

-- @profile-lock
-- @test
INSERT INTO ocdzk SELECT a, b FROM ocdzksrc WHERE false ON CONFLICT (a) DO UPDATE SET b = 1;
SELECT a, b FROM ocdzk ORDER BY a;

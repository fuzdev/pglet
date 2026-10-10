-- The other position of the same control: an unknown `SET` target ahead of the repeat is
-- `42703` too. The pair is what says the `SET` list's verdicts do not interleave — the
-- unknown name wins from **either** side, where the `INSERT` column list's answer depends
-- on which item comes first.
-- @setup
CREATE TABLE ucdb (a INT, b INT);
INSERT INTO ucdb VALUES (0, 0);

-- @expect-error: column "zz" of relation "ucdb" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE ucdb SET zz = 1, a = 1, a = 2;

-- **The sharpest control in the duplicate-name family.** A `SET` list does *not*
-- interleave its two verdicts the way an `INSERT` or `COPY` column list does:
-- `transformUpdateTargetList` resolves every target name during parse analysis and the
-- duplicate check lives in the rewriter, a whole phase later — so an unknown name at item
-- 3 outranks a repeat at items 1 and 2, and `42703` is the answer.
--
-- Its `INSERT` counterpart is the opposite (`INSERT INTO t (a, a, zz)` is `42701`,
-- `insert/duplicate_target_column_beats_a_later_unknown_name_loud.sql`). One shared
-- interleaved helper applied to all three families reports `42701` here and is wrong;
-- pglet passed this file before the gate existed and must keep passing it.
-- @setup
CREATE TABLE ucd (a INT, b INT);
INSERT INTO ucd VALUES (0, 0);

-- @expect-error: column "zz" of relation "ucd" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE ucd SET a = 1, a = 2, zz = 3;

-- The `INSERT … SELECT` member: a silent **success** rather than a write, since the source
-- is empty and nothing lands — `INSERT 0 0` here against 18.6's `42703`. It is
-- row-count-**dependent**, unlike the unreferenced-CTE polarity: seed the source and the
-- eval-time resolver raises `42703` here too (measured), which is what puts this member on
-- the missing plan-time judgement rather than on the clause.
--
-- Its **qualified** spelling is `42P01` on both engines at either row count — the blanket
-- exempted bare names only — which is the control that identifies this as the blanket's.
-- @setup
CREATE TABLE sk_isrc (a INT);
CREATE TABLE sk_itgt (a INT, b INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
INSERT INTO sk_itgt SELECT a, 1 FROM sk_isrc ORDER BY bogus;

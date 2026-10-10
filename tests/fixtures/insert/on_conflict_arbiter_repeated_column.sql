-- An `ON CONFLICT` inference list is a **set**, so a repeated column part collapses and
-- the index still matches: `infer_arbiter_indexes` builds a `Bitmapset` of the plain
-- attributes on each side and compares them with `bms_equal`. pglet compared the two key
-- lists element-wise and answered `42P10`.
--
-- This is the accept polarity of the duplicate-name work beside it, and the two pull in
-- opposite directions on the same spelling: `(a, a)` is a list an `INSERT` column list
-- must start **refusing** and an inference list must start **accepting**.
-- @setup
CREATE TABLE oar (a INT PRIMARY KEY, b INT);
INSERT INTO oar VALUES (1, 1);
INSERT INTO oar VALUES (1, 9) ON CONFLICT (a, a) DO NOTHING;

-- @profile-lock
-- @test
SELECT a, b FROM oar ORDER BY a;

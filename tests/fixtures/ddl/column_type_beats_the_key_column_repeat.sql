-- And against the key-list repeat, which is the highest of the three: the `tableElts`
-- walk still outranks it, because `transformIndexConstraints` runs after it in the same
-- parse-analysis pass. The bad type is written **after** the constraint here, which is
-- the arrangement that separates a whole-list type walk from a per-element interleave.
-- @expect-error: the refused char length outranks the repeated key column
-- @no-profile-lock: the @test is DDL and its setup builds nothing — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE ctbkc (b INT, PRIMARY KEY (b, b), c CHAR(0));

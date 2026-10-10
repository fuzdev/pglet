-- The first of the four persisted-expression sites. Without the verdict the column was
-- defined and the modifier dropped, so `information_schema.columns.column_default` read
-- back `upper('a')` — not what the user wrote — and an insert omitting the column stored
-- `A`. That rendering is a second silent face of the same defect and closes with it.
-- 42809 on both.
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
CREATE TABLE sq_dd1 (c TEXT DEFAULT upper(DISTINCT 'a'));

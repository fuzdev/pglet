-- `name` is in Postgres's string category and implicitly casts to `text`, so a
-- `name`-typed expression feeds the ordinary text operators and functions. The
-- declared types are the point as much as the values: `length` → `int4`,
-- `upper(name)` → `text`, `name || text` → `text` on both engines.
-- @test
SELECT length(current_user) AS len, upper(current_schema) AS up, current_user || '!' AS cat;

-- The accepting direction of the `name` branch rule, which the refusing fixtures beside
-- this one cannot see. `name` is `typcategory` `S` on 18.6 with implicit `pg_cast` rows
-- **both ways** against `text` (`name → text`, `text → name`, `varchar → name`), so a
-- branch set the gate reads as `name` beside a string sibling resolves rather than being
-- refused — and resolves to `name` (19), the fold keeping its first type precisely
-- because the reverse cast is implicit too. `text` is the string category's *preferred*
-- type, so the text-first spelling keeps `text` (25) instead — the order is observable,
-- and each direction pins a different `pg_cast` row. The `varchar` sibling keeps `name`
-- for the *other* reason: `name → varchar` is assignment, so the forward cast the
-- displacement needs is the one that is missing. The `name[]` column is the array
-- spelling of the same step, read element-wise. The `varchar`-written-first order is
-- pinned at `greatest` and `least` only: its `COALESCE`, `CASE` and `ARRAY[…]` spellings
-- declare `1043`, `1043` and `1015` against 18.6's `19`, `19` and `1003`, so a faithful
-- fixture over those is red (`EDGES.md` row 152).
-- @setup
CREATE TABLE ssns (i INT, tx TEXT, vc VARCHAR(8), ta TEXT[]);
INSERT INTO ssns VALUES (1, 'aaa', 'bbb', ARRAY['zzz']);
-- @test
SELECT
  COALESCE((SELECT current_schema), tx) AS coalesced,
  COALESCE(tx, (SELECT current_schema)) AS text_first,
  greatest((SELECT current_schema), tx) AS greatest_of,
  least((SELECT current_schema), vc) AS least_of,
  ARRAY[(SELECT current_schema), tx] AS arrayed,
  COALESCE((SELECT current_schemas(true)), ta) AS coalesced_array,
  greatest(vc, (SELECT current_schema)) AS greatest_varchar_first,
  least(vc, (SELECT current_schema)) AS least_varchar_first
FROM ssns;

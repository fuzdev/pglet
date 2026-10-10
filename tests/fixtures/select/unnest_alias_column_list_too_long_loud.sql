-- A FROM item's column-alias list longer than the relation provides is `42P10` with
-- `parse_relation.c`'s wording — the rule the `(VALUES …) AS k(…)` route already had and
-- this one silently **truncated**, answering one column named `a`.
-- @expect-error: more aliases than the unnest provides
-- @expect-message
-- @test
SELECT * FROM unnest(ARRAY[1, 2]) AS k(a, b);

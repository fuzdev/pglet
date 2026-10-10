-- The literal's conversion at a comparison over a genuinely recursive CTE's column, a door
-- `enum/non_member_literal_*` did not reach: the rebuilt column carried no identity, so
-- `'zzz'` stayed a string and ranked after every member instead of failing to convert.
-- The recursive term's own fold is a second door, held by
-- `with_recursive_list/enum_recursive_term_non_member_literal_loud` (`EDGES.md` row 114).
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'ok'), (2, 'sad'), (3, 'happy');

-- @test
-- @expect-error: 'zzz' is not a member of mood
-- @expect-message
WITH RECURSIVE r(z) AS (
  SELECT m FROM t
  UNION ALL
  SELECT z FROM r WHERE false
)
SELECT 1 AS one FROM r WHERE z > 'zzz';

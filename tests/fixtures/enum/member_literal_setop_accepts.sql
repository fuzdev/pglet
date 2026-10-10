-- The set-operation door's accept polarity, beside its two `non_member_literal_setop_*`
-- refusals: a member label converts where the non-member is refused.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
WITH u AS (SELECT m FROM t UNION ALL SELECT 'happy')
SELECT m::text AS label FROM u ORDER BY label;

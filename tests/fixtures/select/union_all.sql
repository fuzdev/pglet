-- UNION ALL keeps every row from both sides (no dedup), so each of the two rows
-- appears twice.
-- @setup
CREATE TABLE union_all_t (id INT);
INSERT INTO union_all_t VALUES (1), (2);
-- @test
SELECT id FROM union_all_t
UNION ALL
SELECT id FROM union_all_t
ORDER BY id;

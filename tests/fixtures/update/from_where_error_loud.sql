-- An UPDATE … FROM whose WHERE errors now fails loud (the join-WHERE eval error
-- propagates) instead of silently updating 0 rows.
-- @setup
CREATE TABLE uw_tgt (id INT, n INT);
CREATE TABLE uw_src (id INT, k INT);
INSERT INTO uw_tgt VALUES (1, 10);
INSERT INTO uw_src VALUES (1, 5);
-- @expect-error: division by zero in the UPDATE…FROM WHERE propagates (22012)
-- @test
UPDATE uw_tgt t SET n = 99 FROM uw_src s WHERE t.id = s.id AND s.k / 0 = 1;

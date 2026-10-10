-- A mistyped join condition (`ON a.int_id = b.uuid_col`) is `42883`, not a silent
-- empty result. This is the shape the operand-gating audit called out: the join
-- ran, matched nothing, and reported success — indistinguishable from "no data".
-- @setup
CREATE TABLE a (id INT);
CREATE TABLE b (uid UUID);
INSERT INTO a VALUES (1);
INSERT INTO b VALUES ('11111111-1111-1111-1111-111111111111');
-- @test
-- @expect-error: no integer = uuid operator
SELECT a.id, b.uid FROM a JOIN b ON a.id = b.uid;

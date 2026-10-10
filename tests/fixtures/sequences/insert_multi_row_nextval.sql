-- A single multi-row INSERT ... VALUES advances the sequence once per row, so the
-- rows get consecutive values.
-- @setup
CREATE SEQUENCE seq_ins;
CREATE TABLE t_ins (id BIGINT);
INSERT INTO t_ins (id) VALUES (nextval('seq_ins')), (nextval('seq_ins'));
-- @test
SELECT id FROM t_ins ORDER BY id;

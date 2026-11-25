-- Two writes drawing one sequence would take its values in another order than 18.6's
-- row-by-row interleaving, so the statement is refused (`0A000`; 18.6 answers).
-- @setup
CREATE TABLE dms (id SERIAL, v TEXT);
CREATE TABLE dms2 (id INT DEFAULT nextval('dms_id_seq'), v TEXT);
-- @expect-error: two writes drawing one sequence
-- @test
WITH c AS (INSERT INTO dms (v) VALUES ('a'), ('b') RETURNING id) INSERT INTO dms2 (v) SELECT 'x' FROM c;

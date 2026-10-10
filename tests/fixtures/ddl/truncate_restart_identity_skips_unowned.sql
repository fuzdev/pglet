-- ...and the other direction: a sequence a column merely *references* is nobody's
-- dependent, so it keeps counting across the truncate. Reading the column defaults
-- restarts it, which is the over-reset.
-- @setup
CREATE SEQUENCE trisu_seq START WITH 100;
CREATE TABLE trisu_t (x INT DEFAULT nextval('trisu_seq'));
INSERT INTO trisu_t (x) VALUES (DEFAULT);
TRUNCATE trisu_t RESTART IDENTITY;
INSERT INTO trisu_t (x) VALUES (DEFAULT);
-- @test
SELECT x FROM trisu_t;

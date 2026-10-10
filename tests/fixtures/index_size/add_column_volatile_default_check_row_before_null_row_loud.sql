-- The rewrite checks each row NOT NULL then CHECK before it reads the next: the first row's
-- 1 fails the CHECK before the second row's NULL is reached.
-- @setup
CREATE TABLE t (id int);
INSERT INTO t VALUES (1), (2);
CREATE SEQUENCE s;
-- @test
-- @expect-error: the first row's CHECK before the second row's NULL
-- @expect-message
-- @profile-lock
ALTER TABLE t ADD COLUMN b int NOT NULL CHECK (b > 1) DEFAULT NULLIF(nextval('s'), 2);

-- And **which** enum the enclosing chain answered with: a second enum beside the first
-- has no operator either (`EDGES.md` row 115), so an arm that found an enum and lost its
-- name accepts this.
-- @expect-error: two different enums through an enclosing sub-plan column
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE t (id INT, m mood NOT NULL, y m3 NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok'), (2, 'ok', 'happy'), (3, 'happy', 'sad');

-- @test
SELECT m IN (SELECT y FROM (VALUES (1)) q) FROM t;

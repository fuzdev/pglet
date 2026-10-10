-- Under `NULLS NOT DISTINCT`, Postgres still treats NULL and the empty string
-- '' as DISTINCT values — only NULL keys collide with each other, not with ''.
-- A NULL-keyed row and an ''-keyed row both insert cleanly. (Regression for the
-- index-key bug where NULL and '' both encoded to the empty byte string and so
-- folded to the same index blob, falsely rejecting the second insert.)
-- @setup
CREATE TABLE nnd_null_empty (id INT, tag TEXT);
CREATE UNIQUE INDEX ux_nnd_null_empty ON nnd_null_empty (tag) NULLS NOT DISTINCT;
INSERT INTO nnd_null_empty VALUES (1, NULL);
INSERT INTO nnd_null_empty VALUES (2, '');

-- @test
SELECT id, tag FROM nnd_null_empty ORDER BY id;

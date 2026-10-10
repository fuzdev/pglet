-- `OWNED BY NONE` detaches. What this pins is that the statement is accepted and the
-- counter untouched; the *detach* itself is observable through
-- `TRUNCATE ... RESTART IDENTITY`, which restarts only owned sequences — see
-- `truncate_restart_identity_after_owned_by_none.sql`.
-- @setup
CREATE TABLE anone_host (a INT);
CREATE SEQUENCE anone_seq START WITH 4;
ALTER SEQUENCE anone_seq OWNED BY anone_host.a;
ALTER SEQUENCE anone_seq OWNED BY NONE;
-- @test
SELECT nextval('anone_seq');

-- `id int GENERATED ALWAYS AS IDENTITY` auto-assigns 1, 2, …; `OVERRIDING SYSTEM
-- VALUE` forces a literal (99) without advancing the sequence; a `DEFAULT` value
-- draws the next sequence number (3, not 100). The 428C9 rejection of a bare
-- explicit value is covered by identity_always_insert_loud.
-- @setup
CREATE TABLE t (id int GENERATED ALWAYS AS IDENTITY, name text);
INSERT INTO t (name) VALUES ('a'), ('b');
INSERT INTO t (id, name) OVERRIDING SYSTEM VALUE VALUES (99, 'forced');
INSERT INTO t (id, name) VALUES (DEFAULT, 'd');
-- @test
SELECT id, name FROM t ORDER BY id;

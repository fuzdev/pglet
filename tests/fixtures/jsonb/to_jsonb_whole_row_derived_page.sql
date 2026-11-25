-- A page of rows from a derived table, each one JSON object, with the page's bounds
-- bound as parameters.
-- @setup
CREATE TABLE tj_page (id TEXT, data BYTEA, note TEXT);
INSERT INTO tj_page VALUES ('a', '\xdeadbeef', 'x'), ('b', NULL, 'y'), ('c', '\x00', 'z');
-- @param $1 int8 2
-- @param $2 int8 1
-- @test
SELECT to_jsonb(t)::text FROM (SELECT id, ('<' || octet_length("data") || ' bytes>') AS "data", note
                               FROM tj_page ORDER BY id LIMIT $1 OFFSET $2) t;

-- @setup
CREATE TABLE trunc_seq (id SERIAL, v TEXT);
INSERT INTO trunc_seq (v) VALUES ('a'), ('b');
TRUNCATE trunc_seq RESTART IDENTITY;
INSERT INTO trunc_seq (v) VALUES ('c');

-- @test
SELECT id, v FROM trunc_seq ORDER BY id;

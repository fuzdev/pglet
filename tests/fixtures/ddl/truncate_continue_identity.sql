-- @setup
CREATE TABLE trunc_cont (id SERIAL, v TEXT);
INSERT INTO trunc_cont (v) VALUES ('a'), ('b');
TRUNCATE trunc_cont;
INSERT INTO trunc_cont (v) VALUES ('c');

-- @test
SELECT id, v FROM trunc_cont ORDER BY id;

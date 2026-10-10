-- Tri-state optional filter `($1::text IS NULL OR kind = $1::text)` with the
-- parameter bound to NULL — the filter is disabled and all rows return.
-- @setup
CREATE TABLE p_opt (id int4, kind text);
INSERT INTO p_opt (id, kind) VALUES (1, 'a'), (2, 'b'), (3, 'a');
-- @param $1 text NULL
-- @test
SELECT id FROM p_opt WHERE ($1::text IS NULL OR kind = $1::text) ORDER BY id;

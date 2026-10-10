-- `refs @> $1` with a bound text[] parameter (the spine's refs containment).
-- @setup
CREATE TABLE p_refs (id int4, refs text[]);
INSERT INTO p_refs (id, refs) VALUES (1, '{a,b}'), (2, '{c}'), (3, '{a,c}');
-- @param $1 text[] {a}
-- @test
SELECT id FROM p_refs WHERE refs @> $1 ORDER BY id;

-- A chain of `||` over a `jsonb` reads each untyped side as `jsonb`, merging left to right.
-- @setup
CREATE TABLE jch (j JSONB);
INSERT INTO jch VALUES ('{"k": 1}');
-- @test
SELECT j || '{"a": 1}' || '{"b": 2}' AS col_chain,
       '{"a": 1}'::jsonb || '{"b": 1}'::jsonb || '{"c": 1}' AS cast_chain FROM jch;

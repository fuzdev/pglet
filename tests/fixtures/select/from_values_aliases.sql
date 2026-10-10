-- @setup

-- @test
-- a VALUES list as a derived table with an explicit column-alias list
SELECT id, name FROM (VALUES (1, 'apple'), (2, 'banana'), (3, 'cherry')) AS t(id, name) ORDER BY id;

-- @setup

-- @test
SELECT role, scope FROM unnest(ARRAY['editor', 'viewer'], ARRAY[10, 20]) AS p(role, scope) ORDER BY scope;

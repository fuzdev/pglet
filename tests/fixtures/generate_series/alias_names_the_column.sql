-- A bare alias names the relation and, the item holding one function, its column too
-- (`chooseScalarFunctionAlias`): `g` and `g.g` both read it.
-- @test
SELECT g, g.g FROM generate_series(1, 3) AS g;

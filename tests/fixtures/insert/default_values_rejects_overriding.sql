-- The other half of the alternative's shape: `insert_rest`'s `DEFAULT VALUES` carries
-- no `OVERRIDING` either, so `42601` at the `DEFAULT` token on 18.6. The target here
-- is a `GENERATED ALWAYS` identity, which is what makes the fixture discriminate — an
-- engine that simply ignored the clause would **succeed**, drawing the identity
-- default, rather than answering some other error.

-- @expect-error: DEFAULT VALUES takes no OVERRIDING clause (42601)
-- @setup
CREATE TABLE dv_ov (id INT GENERATED ALWAYS AS IDENTITY, v TEXT);

-- @test
INSERT INTO dv_ov OVERRIDING SYSTEM VALUE DEFAULT VALUES;

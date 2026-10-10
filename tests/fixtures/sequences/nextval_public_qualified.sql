-- A `public.`-qualified sequence name resolves to the same slot as the bare name
-- (the CREATE SEQUENCE / DEFAULT nextval reduction path).
-- @setup
CREATE SEQUENCE seq_pub;
-- @test
SELECT nextval('public.seq_pub');

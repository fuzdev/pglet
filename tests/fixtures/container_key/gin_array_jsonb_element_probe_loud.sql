-- The same over a `text[]` column probed with a `jsonb[]`.
-- @profile-lock
-- @setup
CREATE TABLE cky_g (id INT, t TEXT[]);
CREATE INDEX cky_g_ix ON cky_g USING gin (t);
INSERT INTO cky_g (t, id) VALUES ('{a,1}', 1);
-- @test
-- @expect-error: text[] @> jsonb[] has no operator (42883)
SELECT id FROM cky_g WHERE t @> ARRAY['1'::jsonb];

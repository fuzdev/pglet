-- The `ON CONFLICT DO UPDATE` write path is the store finalizer's too: `EXCLUDED.a`
-- carries the proposed row's value and the update rounds it.
-- @setup
CREATE TABLE tpc (id int PRIMARY KEY, a timestamp(3));
INSERT INTO tpc VALUES (1, '2020-01-02 00:00:00');
INSERT INTO tpc VALUES (1, '2020-01-02 03:04:05.9999') ON CONFLICT (id) DO UPDATE SET a = EXCLUDED.a;
-- @test
SELECT id, a FROM tpc;

-- `ON CONFLICT DO UPDATE` reads the instant the proposed row was stamped with: the
-- `excluded` row's `DEFAULT now()`, the `SET`'s `now()` and the `RETURNING`'s are one
-- value, on the inserted row and the updated ones alike.
-- @setup
CREATE TABLE coc (id INT PRIMARY KEY, created TIMESTAMPTZ DEFAULT now(), updated TIMESTAMPTZ, seen TIMESTAMPTZ);
INSERT INTO coc (id) VALUES (1), (2);
-- @test
INSERT INTO coc (id) VALUES (1), (2), (3)
ON CONFLICT (id) DO UPDATE SET updated = now(), seen = excluded.created
RETURNING id, updated = seen AS set_is_excluded, coalesce(updated, created) = now() AS stamp_is_returning,
          created = now() AS created_this_transaction;

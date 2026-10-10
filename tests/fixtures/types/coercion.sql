-- @setup
CREATE TABLE coerce_test (id INT, price FLOAT, name TEXT, active BOOL);
INSERT INTO coerce_test VALUES (1, 19.99, 'Widget', true);
INSERT INTO coerce_test VALUES (2, 9.99, 'Gadget', false);
INSERT INTO coerce_test VALUES (3, 29.99, 'Gizmo', true);

-- @test
-- String literal '1' should be coerced to INT when comparing to INT column
SELECT * FROM coerce_test WHERE id = '1';

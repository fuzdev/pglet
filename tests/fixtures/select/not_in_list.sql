-- @setup
CREATE TABLE products (id INT, name TEXT, category TEXT);
INSERT INTO products VALUES (1, 'Widget', 'tools'), (2, 'Gadget', 'electronics'), (3, 'Gizmo', 'electronics'), (4, 'Thing', 'other');

-- @test
SELECT name, category FROM products WHERE category NOT IN ('tools', 'electronics') ORDER BY name;

-- ===========================================
-- Add Resource Group Management Permissions
-- ===========================================
-- Date: 2026-01-13
-- Purpose: Add menu and API permissions for resource group management

-- ============ Menu Permissions ============
INSERT OR IGNORE INTO permissions (code, name, type, resource, action, description) VALUES
('menu:cluster-ops:resource-groups', 'Resource Group Management', 'menu', 'cluster-ops:resource-groups', 'view', 'View resource group management');

-- ============ API Permissions ============
INSERT OR IGNORE INTO permissions (code, name, type, resource, action, description) VALUES
-- Resource Group CRUD
('api:resource-groups:list', 'List resource groups', 'api', 'resource-groups', 'list', 'GET /api/clusters/resource-groups'),
('api:resource-groups:get', 'Get resource group details', 'api', 'resource-groups', 'get', 'GET /api/clusters/resource-groups/:name'),
('api:resource-groups:create', 'Create resource group', 'api', 'resource-groups', 'create', 'POST /api/clusters/resource-groups'),
('api:resource-groups:update', 'Update resource group', 'api', 'resource-groups', 'update', 'PUT /api/clusters/resource-groups/:name'),
('api:resource-groups:delete', 'Delete resource group', 'api', 'resource-groups', 'delete', 'DELETE /api/clusters/resource-groups/:name'),

-- Resource Group Monitoring
('api:resource-groups:usage', 'Get resource group usage', 'api', 'resource-groups', 'usage', 'GET /api/clusters/resource-groups/usage'),

-- Resource Group Analysis
('api:resource-groups:analysis', 'Resource usage analysis', 'api', 'resource-groups', 'analysis', 'GET /api/clusters/resource-groups/analysis');

-- ============ Set Parent Relationships ============
-- Set menu parent
UPDATE permissions
SET parent_id = (SELECT id FROM permissions WHERE code = 'menu:cluster-ops')
WHERE code = 'menu:cluster-ops:resource-groups';

-- Set API parents
UPDATE permissions
SET parent_id = (SELECT id FROM permissions WHERE code = 'menu:cluster-ops:resource-groups')
WHERE code IN (
    'api:resource-groups:list',
    'api:resource-groups:get',
    'api:resource-groups:create',
    'api:resource-groups:update',
    'api:resource-groups:delete',
    'api:resource-groups:usage',
    'api:resource-groups:analysis'
);

-- ============ Grant to Admin Roles ============
-- Grant to admin role
INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
SELECT (SELECT id FROM roles WHERE code='admin'), id FROM permissions
WHERE code IN (
    'menu:cluster-ops:resource-groups',
    'api:resource-groups:list',
    'api:resource-groups:get',
    'api:resource-groups:create',
    'api:resource-groups:update',
    'api:resource-groups:delete',
    'api:resource-groups:usage',
    'api:resource-groups:analysis'
);

-- Grant to super_admin role
INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
SELECT (SELECT id FROM roles WHERE code='super_admin'), id FROM permissions
WHERE code IN (
    'menu:cluster-ops:resource-groups',
    'api:resource-groups:list',
    'api:resource-groups:get',
    'api:resource-groups:create',
    'api:resource-groups:update',
    'api:resource-groups:delete',
    'api:resource-groups:usage',
    'api:resource-groups:analysis'
);

-- ============ Auto-grant Parent Menu ============
-- Grant menu:cluster-ops to roles that have resource-groups permission
INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
SELECT DISTINCT rp.role_id, (SELECT id FROM permissions WHERE code = 'menu:cluster-ops')
FROM role_permissions rp
JOIN permissions p ON rp.permission_id = p.id
WHERE p.code = 'menu:cluster-ops:resource-groups';

import { Component, OnInit, OnDestroy, Input, Output, EventEmitter, ViewChild, TemplateRef } from '@angular/core';
import { Subject } from 'rxjs';
import { takeUntil } from 'rxjs/operators';
import { LocalDataSource } from 'angular2-smart-table';
import { PermissionRequestService } from '../../../../@core/data/permission-request.service';
import { DbUserPermissionDto } from '../../../../@core/data/permission-request.model';
import { NbDialogService, NbToastrService } from '@nebular/theme';

/**
 * Standard Permission Dashboard Component
 * 我的权限 - 展示当前集群数据库用户的权限列表
 *
 * 设计说明：
 * 1. 权限来源于 OLAP 引擎（StarRocks/Doris），通过 SHOW GRANTS 查询
 * 2. 权限分类：
 *    - 角色权限 (ROLE): 用户被授予的角色
 *    - 全局权限 (GLOBAL/SYSTEM): 系统级别权限，如 BLACKLIST, NODE 等
 *    - 数据库权限 (DATABASE): 数据库级别权限
 *    - 表级权限 (TABLE): 表级别权限
 * 3. 权限只能通过"权限申请"流程撤销，不能直接删除
 */

interface PermissionRecord extends DbUserPermissionDto {
  risk_level?: 'low' | 'medium' | 'high';
}

@Component({
  standalone: false,
  selector: 'ngx-permission-dashboard-standard',
  templateUrl: './permission-dashboard-standard.component.html',
  styleUrls: ['./permission-dashboard-standard.component.scss'],
})
export class PermissionDashboardStandardComponent implements OnInit, OnDestroy {
  @Input() refresh$: Subject<void>;
  @Input() clusterId: number;
  @Output() revokePermission = new EventEmitter<PermissionRecord>();
  @Output() switchToRequest = new EventEmitter<{type: string, permission?: PermissionRecord}>();

  // 状态
  loading = false;
  permissions: PermissionRecord[] = [];
  filteredPermissions: PermissionRecord[] = [];
  source: LocalDataSource = new LocalDataSource();

  // ng2-smart-table 配置 - 仿照节点管理，使用标准编辑按钮查看详情
  settings = {
    mode: 'external',
    hideSubHeader: false,
    noDataMessage: 'No permission data',
    actions: {
      columnTitle: 'Actions',
      add: false,
      edit: true,  // 使用编辑按钮作为查看详情
      delete: false,
      position: 'right',
    },
    edit: {
      editButtonContent: '<i class="nb-search"></i>',
    },
    pager: {
      display: true,
      perPage: 15,
    },
    columns: {
      privilege_type: {
        title: 'Permission Type',
        type: 'string',
      },
      resource_type: {
        title: 'Resource Type',
        type: 'string',
      },
      resource_path: {
        title: 'Resource Path',
        type: 'string',
      },
      granted_role: {
        title: 'Granted Role',
        type: 'string',
        valuePrepareFunction: (cell: string) => {
          return cell || 'Direct grant';
        },
      },
      risk_level_display: {
        title: 'Risk Level',
        type: 'string',
      },
    },
  };

  // 统计
  stats = {
    totalRoles: 0,
    globalPermissions: 0,
    dbPermissions: 0,
    tablePermissions: 0,
  };

  selectedPermission: PermissionRecord | null = null;
  @ViewChild('detailDialog') detailDialog: TemplateRef<any>;

  // 角色权限详情
  rolePermissions: DbUserPermissionDto[] = [];
  loadingRolePermissions = false;

  private destroy$ = new Subject<void>();

  constructor(
    private permissionService: PermissionRequestService,
    private toastr: NbToastrService,
    private dialogService: NbDialogService,
  ) {}

  ngOnInit(): void {
    if (this.refresh$) {
      this.refresh$.pipe(takeUntil(this.destroy$)).subscribe(() => {
        this.loadPermissions();
      });
    }
    this.loadPermissions();
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
  }

  /**
   * 加载权限列表
   */
  loadPermissions(): void {
    this.loading = true;

    this.permissionService.listMyDbPermissions().subscribe({
      next: (permissions: DbUserPermissionDto[]) => {
        this.permissions = this.enhancePermissions(permissions);
        this.updatePermissionsDisplay();
        this.calculateStats();
        this.loading = false;
      },
      error: (err) => {
        console.error('Failed to load permissions:', err);
        this.toastr.danger('Failed to load permission list', 'Error');
        this.loading = false;
      },
    });
  }

  /**
   * 增强权限数据
   */
  private enhancePermissions(permissions: DbUserPermissionDto[]): PermissionRecord[] {
    return permissions.map(p => {
      const riskLevel = this.calculateRiskLevel(p.privilege_type, p.resource_type);
      return {
        ...p,
        risk_level: riskLevel,
        risk_level_display: this.getRiskLabel(riskLevel),
      };
    });
  }

  /**
   * 计算风险等级
   * 基于权限类型和资源范围判断风险
   */
  private calculateRiskLevel(privilege: string, resourceType: string): 'low' | 'medium' | 'high' {
    const highRiskPrivileges = ['DELETE', 'DROP', 'GRANT', 'ADMIN', 'NODE', 'BLACKLIST', 'ALL'];
    const mediumRiskPrivileges = ['INSERT', 'UPDATE', 'CREATE', 'ALTER', 'LOAD'];

    // 全局权限风险更高
    if (resourceType === 'GLOBAL' || resourceType === 'SYSTEM') {
      return 'high';
    }

    if (highRiskPrivileges.some(p => privilege.toUpperCase().includes(p))) {
      return 'high';
    }
    if (mediumRiskPrivileges.some(p => privilege.toUpperCase().includes(p))) {
      return 'medium';
    }
    return 'low';
  }

  /**
   * 计算统计数据
   *
   * 权限分类说明：
   * - 角色数：用户被授予的角色数量（resource_type === 'ROLE'）
   * - 全局权限：系统级别权限（resource_type === 'GLOBAL' 或 'SYSTEM' 或 'CATALOG'）
   * - 数据库权限：数据库级别权限（resource_type === 'DATABASE'）
   * - 表级权限：表级别权限（resource_type === 'TABLE'）
   */
  private calculateStats(): void {
    // 角色数量
    this.stats.totalRoles = this.permissions.filter(p =>
      p.resource_type === 'ROLE' || p.privilege_type === 'ROLE'
    ).length;

    // 全局权限（GLOBAL, SYSTEM, CATALOG 级别）
    this.stats.globalPermissions = this.permissions.filter(p =>
      p.resource_type === 'GLOBAL' ||
      p.resource_type === 'SYSTEM' ||
      p.resource_type === 'CATALOG'
    ).length;

    // 数据库权限
    this.stats.dbPermissions = this.permissions.filter(p =>
      p.resource_type === 'DATABASE'
    ).length;

    // 表级权限
    this.stats.tablePermissions = this.permissions.filter(p =>
      p.resource_type === 'TABLE'
    ).length;
  }

  /**
   * 更新权限列表显示
   */
  private updatePermissionsDisplay(): void {
    this.filteredPermissions = [...this.permissions];
    this.source.load(this.filteredPermissions);
  }

  /**
   * 编辑行事件 - 用于查看详情
   */
  onEditRow(event: any): void {
    const permission = event.data as PermissionRecord;
    this.viewPermissionDetail(permission);
  }

  /**
   * 查看权限详情
   */
  viewPermissionDetail(permission: PermissionRecord): void {
    this.selectedPermission = permission;
    this.rolePermissions = [];

    if (this.isRolePermission) {
      this.loadRolePermissions(permission.resource_path);
    }

    this.dialogService.open(this.detailDialog);
  }

  /**
   * 判断当前选中的权限是否是角色类型
   */
  get isRolePermission(): boolean {
    return this.selectedPermission?.resource_type === 'ROLE' ||
           this.selectedPermission?.privilege_type === 'ROLE';
  }

  /**
   * 加载角色的具体权限
   */
  private loadRolePermissions(roleName: string): void {
    this.loadingRolePermissions = true;
    this.permissionService.listRolePermissions(roleName).subscribe({
      next: (permissions) => {
        this.rolePermissions = permissions;
        this.loadingRolePermissions = false;
      },
      error: (err) => {
        console.error('Failed to load role permissions:', err);
        this.loadingRolePermissions = false;
      },
    });
  }

  requestRevoke(permission: PermissionRecord): void {
    // 发送事件给父组件，切换到权限申请Tab并预填信息
    this.switchToRequest.emit({
      type: 'revoke_permission',
      permission: permission,
    });

    this.toastr.info(
      'Please fill in the revocation reason on the permission request page and submit',
      'Go to Permission Request'
    );
  }

  // 辅助方法
  getRiskLabel(risk: string): string {
    switch (risk) {
      case 'high': return 'High Risk';
      case 'medium': return 'Medium Risk';
      case 'low': return 'Low Risk';
      default: return 'Low Risk';
    }
  }

  getRiskColor(risk: string): string {
    switch (risk) {
      case 'high': return 'danger';
      case 'medium': return 'warning';
      case 'low': return 'success';
      default: return 'basic';
    }
  }

  /**
   * 获取权限类型说明
   */
  getPrivilegeTypeDescription(type: string): string {
    const descriptions: {[key: string]: string} = {
      'SELECT': 'Query data',
      'INSERT': 'Insert data',
      'UPDATE': 'Update data',
      'DELETE': 'Delete data',
      'CREATE': 'Create objects',
      'DROP': 'Drop objects',
      'ALTER': 'Alter objects',
      'GRANT': 'Grant permissions',
      'USAGE': 'Use resources',
      'ROLE': 'Role grant',
      'BLACKLIST': 'Blacklist management',
      'NODE': 'Node management',
      'ADMIN': 'Administrator privileges',
      'ALL': 'All privileges',
    };
    return descriptions[type.toUpperCase()] || type;
  }

  /**
   * 获取权限详细描述（用于详情对话框）
   */
  getPrivilegeDescription(type: string): string {
    const descriptions: {[key: string]: string} = {
      'SELECT': 'Allows running SELECT queries on the specified resource to read data from tables. This is the most basic data access permission.',
      'INSERT': 'Allows inserting new data into the specified table. Commonly used for data loading and ETL jobs.',
      'UPDATE': 'Allows modifying existing records in the specified table. Grant with care, it may affect data integrity.',
      'DELETE': 'Allows deleting records from the specified table. High-risk operation, grant it only to those who need it.',
      'CREATE TABLE': 'Allows creating new tables in the specified database.',
      'CREATE VIEW': 'Allows creating views in the specified database.',
      'CREATE DATABASE': 'Allows creating new databases.',
      'CREATE FUNCTION': 'Allows creating user-defined functions (UDFs).',
      'CREATE MATERIALIZED VIEW': 'Allows creating materialized views to speed up queries.',
      'DROP': 'Allows dropping the specified database objects (tables, views, etc.). High-risk operation, dropped data cannot be recovered.',
      'ALTER': 'Allows modifying the structure of the specified object, such as adding columns or changing table properties.',
      'GRANT': 'Allows granting the permissions you own to other users or roles. Users with this permission can distribute permissions.',
      'USAGE': 'Allows using the specified catalog or resource. This is the base permission for accessing external data sources.',
      'ROLE': 'Indicates that the user has been granted a role, which contains a set of predefined permissions.',
      'BLACKLIST': 'Allows managing the SQL blacklist, adding or removing SQL patterns that must not run. This is a system administration permission used to block specific dangerous queries.',
      'NODE': 'Allows managing cluster nodes, including adding and removing BE/CN nodes. This is a cluster management permission.',
      'ADMIN': 'Administrator permission, granting full control over the system.',
      'ALL': 'Grants every permission on the specified resource, equivalent to full control over it.',
      'EXPORT': 'Allows exporting table data to external storage.',
      'LOAD': 'Allows data load operations such as Stream Load and Broker Load.',
      'IMPERSONATE': 'Allows impersonating other users to perform operations. High-risk permission, grant it with care.',
      'OPERATE': 'Allows operational actions such as killing queries and setting system variables.',
      'CREATE RESOURCE': 'Allows creating external resources such as Spark and Hive resources.',
      'CREATE RESOURCE GROUP': 'Allows creating resource groups for resource isolation and management.',
      'CREATE GLOBAL FUNCTION': 'Allows creating global functions.',
      'CREATE STORAGE VOLUME': 'Allows creating storage volumes (compute-storage separation mode).',
      'FILE': 'Allows performing file related operations.',
      'PLUGIN': 'Allows managing plugins.',
      'REPOSITORY': 'Allows managing backup repositories.',
    };
    return descriptions[type.toUpperCase()] || `${type} allows performing related operations.`;
  }

  /**
   * 获取权限使用场景
   */
  getPrivilegeUsage(type: string): string {
    const usages: {[key: string]: string} = {
      'SELECT': 'Data analysis, reporting queries, BI tool connections',
      'INSERT': 'Data loading, ETL jobs, real-time writes',
      'UPDATE': 'Data corrections, status updates',
      'DELETE': 'Data cleanup, expired data removal',
      'DROP': 'Table rebuilds, test data cleanup',
      'ALTER': 'Table schema changes, adding indexes',
      'GRANT': 'Permission management, team permission distribution',
      'USAGE': 'Accessing external catalogs (such as Hive and Iceberg)',
      'BLACKLIST': 'Blocking dangerous SQL, preventing resource abuse',
      'NODE': 'Cluster scaling, node maintenance',
      'ADMIN': 'System administration, emergency operations',
      'ALL': 'Development and test environments, administrator accounts',
      'EXPORT': 'Data backup, data migration',
      'LOAD': 'Bulk data loading, data synchronization',
      'OPERATE': 'Query management, system tuning',
    };
    return usages[type.toUpperCase()] || '';
  }

  /**
   * 获取资源范围描述
   */
  getResourceScopeDescription(resourceType: string, resourcePath: string): string {
    switch (resourceType?.toUpperCase()) {
      case 'GLOBAL':
      case 'SYSTEM':
        return 'Entire cluster, affects all databases and tables';
      case 'CATALOG':
        return resourcePath === '*' ? 'All catalogs' : `Catalog: ${resourcePath}`;
      case 'DATABASE':
        return resourcePath?.endsWith('.*') 
          ? `All tables in database ${resourcePath.replace('.*', '')}`
          : `Database: ${resourcePath}`;
      case 'TABLE':
        return `Table: ${resourcePath}`;
      case 'ROLE':
        return `All permissions contained in role ${resourcePath}`;
      default:
        return resourcePath || 'Unknown scope';
    }
  }

  /**
   * 获取资源类型说明
   */
  getResourceTypeDescription(type: string): string {
    const descriptions: {[key: string]: string} = {
      'GLOBAL': 'Global level - affects the entire cluster',
      'SYSTEM': 'System level - system administration permissions',
      'CATALOG': 'Catalog level - data catalog permissions',
      'DATABASE': 'Database level - permissions on a specific database',
      'TABLE': 'Table level - permissions on a specific table',
      'ROLE': 'Role - permissions obtained through a role',
    };
    return descriptions[type.toUpperCase()] || type;
  }
}

import { Component, OnInit } from '@angular/core';
import { Router, NavigationEnd } from '@angular/router';
import { NbMenuService } from '@nebular/theme';
import { filter, map } from 'rxjs/operators';

import { MENU_ITEMS } from './pages-menu';
import { AuthService } from '../@core/data/auth.service';
import { TabService } from '../@core/services/tab.service';
import { MenuFilterService } from '../@core/services/menu-filter.service';
import { PermissionService } from '../@core/data/permission.service';

@Component({
  standalone: false,
  selector: 'ngx-pages',
  styleUrls: ['pages.component.scss'],
  template: `
    <ngx-one-column-layout>
      <nb-menu [items]="menu" tag="menu" (itemClick)="onMenuClick($event)"></nb-menu>
      <router-outlet></router-outlet>
    </ngx-one-column-layout>
  `,
})
export class PagesComponent implements OnInit {
  menu = MENU_ITEMS;

  constructor(
    private menuService: NbMenuService,
    private authService: AuthService,
    private router: Router,
    private tabService: TabService,
    private menuFilterService: MenuFilterService,
    private permissionService: PermissionService
  ) {}

  ngOnInit() {
    // Filter menu items based on permissions
    this.permissionService.permissions$.subscribe(() => {
      this.menu = this.menuFilterService.filterMenuItems(MENU_ITEMS);
    });

    // Initialize permissions if not already initialized
    if (this.authService.isAuthenticated()) {
      this.permissionService.initPermissions().subscribe();
    }

    // Listen to menu item clicks
    this.menuService.onItemClick()
      .pipe(
        filter(({ tag }) => tag === 'menu'),
        map(({ item }) => item)
      )
      .subscribe(item => {
        if (item.title === 'Log out') {
          this.authService.logout();
        }
      });

    // Listen to route changes and add tabs
    // Only handle route changes when navigation is triggered by menu or direct URL
    this.router.events
      .pipe(filter(event => event instanceof NavigationEnd))
      .subscribe((event: NavigationEnd) => {
        // Skip if this is triggered by tab switching
        // Check if the URL matches any existing active tab (使用规范化比较)
        const activeTab = this.tabService.getActiveTab();
        if (activeTab) {
          // 规范化比较 URL（处理编码问题）
          const normalizedEventUrl = this.normalizeUrl(event.url);
          const normalizedActiveTabUrl = this.normalizeUrl(activeTab.url);
          if (normalizedActiveTabUrl === normalizedEventUrl) {
            // This navigation is from tab switching, skip adding new tab
            return;
          }
        }
        this.handleRouteChange(event.url);
      });
  }

  /**
   * 处理路由变化，自动添加Tab
   */
  private handleRouteChange(url: string): void {
    // 跳过登录页面
    if (url.includes('/auth/')) {
      return;
    }

    // 对于带查询参数的 system 路由，优先使用 inferTitleFromUrl 获取更具体的标题
    let title: string | null = null;
    if (url.includes('/starrocks/system') && url.includes('?')) {
      title = this.inferTitleFromUrl(url);
    }

    // 如果没有通过 URL 推断获得标题，尝试查找菜单项
    if (!title) {
      const menuItem = this.findMenuItemByUrl(url);
      if (menuItem) {
        title = menuItem.title;
      }
    }

    // 如果还是没有标题，尝试从 URL 推断
    if (!title) {
      title = this.inferTitleFromUrl(url);
    }

    // 如果获得了标题，添加 Tab
    if (title) {
      const tabId = this.generateTabId(title);
      const icon = this.getIconForUrl(url);
      // 路由变化时不再触发导航（因为已经在目标路由了）
      this.tabService.addTab({
        id: tabId,
        title: title,
        url: url,
        closable: true,
        pinned: false,
        icon: icon  // Add icon to tab
      }, false);
    }
  }

  /**
   * 规范化 URL，用于比较（处理编码问题）
   */
  private normalizeUrl(url: string): string {
    try {
      let decoded = url;
      try {
        decoded = decodeURIComponent(url);
      } catch (e) {
        decoded = url;
      }
      
      const [path, queryString] = decoded.split('?');
      
      let normalizedPath = path.replace(/\/+$/, '');
      if (!normalizedPath.startsWith('/')) {
        normalizedPath = '/' + normalizedPath;
      }
      
      if (queryString) {
        try {
          const params = new URLSearchParams(queryString);
          const pairs: string[][] = [];
          params.forEach((value, key) => pairs.push([key, value]));
          pairs.sort((a, b) => a[0].localeCompare(b[0]));
          const normalizedParams = new URLSearchParams(pairs);
          return normalizedPath + '?' + normalizedParams.toString();
        } catch (e) {
          return normalizedPath + '?' + queryString;
        }
      }
      
      return normalizedPath;
    } catch (e) {
      // 如果所有处理都失败，返回原始 URL
      return url;
    }
  }

  /**
   * 从URL推断标题
   */
  private inferTitleFromUrl(url: string): string | null {
    // 先解码 URL（处理可能的编码情况）
    try {
      url = decodeURIComponent(url);
    } catch (e) {
      // 如果解码失败，使用原始 URL
    }
    
    // 先分离路径和查询参数
    const [path, queryString] = url.split('?');
    const urlSegments = path.split('/').filter(segment => segment);
    
    // 处理StarRocks相关路由
    if (urlSegments.includes('starrocks')) {
      const lastSegment = urlSegments[urlSegments.length - 1];
      const prevSegment = urlSegments[urlSegments.length - 2] || '';
      
      // 特殊处理 system 路由，从查询参数中提取功能名称
      if (lastSegment === 'system' && queryString) {
        // 处理可能的编码查询参数
        let decodedQueryString = queryString;
        try {
          decodedQueryString = decodeURIComponent(queryString);
        } catch (e) {
          // 如果解码失败，使用原始查询字符串
        }
        const params = new URLSearchParams(decodedQueryString);
        const functionName = params.get('function');
        if (functionName) {
          // 映射功能名称到中文标题（确保与 system-management.component.ts 中的定义一致）
          // 基于 StarRocks 官方支持的 PROC 路径
          const functionTitleMap: { [key: string]: string } = {
            'backends': 'Backend Node Information (BE)',
            'compute_nodes': 'Compute Node Information (CN)',
            'frontends': 'Frontend Node Information',
            'brokers': 'Broker Node Information',
            'statistic': 'Statistics',
            'monitor': 'Monitoring Information',
            'cluster_balance': 'Cluster Balance',
            'historical_nodes': 'Historical Nodes',
            'dbs': 'Database Information',
            'catalog': 'Catalog Information',
            'transactions': 'Transaction Information',
            'routine_loads': 'Routine Load Jobs',
            'stream_loads': 'Stream Load Jobs',
            'load_error_hub': 'Load Error Information',
            'tasks': 'Task List',
            'replications': 'Replication Jobs',
            'current_queries': 'Current Queries',
            'global_current_queries': 'Global Current Queries',
            'current_backend_instances': 'Current Backend Instances',
            'resources': 'Resource Information',
            'warehouses': 'Warehouse Information',
            'compactions': 'Compaction Jobs',
            'colocation_group': 'Colocation Group',
            'jobs': 'Job Information',
            'meta_recovery': 'Metadata Recovery'
          };
          return functionTitleMap[functionName] || `Feature Cards: ${functionName}`;
        }
      }
      
      // 当路径为 /pages/starrocks/clusters/:id 时，避免使用数字作为标题
      if (prevSegment === 'clusters' && /^\d+$/.test(lastSegment)) {
        return 'Cluster Detail';
      }

      // 映射URL段到中文标题
      const titleMap: { [key: string]: string } = {
        'dashboard': 'Cluster List',
        'overview': 'Cluster Overview',
        'frontends': 'Frontend Nodes',
        'backends': 'Backend Nodes',
        'execution': 'Running Queries',
        'profiles': 'Profiles',
        'audit-logs': 'Audit Log',
        'materialized-views': 'Materialized Views',
        'system': 'Feature Cards',  // 对应 menu:system-functions
        'sessions': 'Session Management',
        'variables': 'Variable Management',
        'clusters': 'Cluster Management',
        'new': 'New Cluster',
        'edit': 'Edit Cluster'
      };
      
      return titleMap[lastSegment] || lastSegment;
    }
    
    return null;
  }

  /**
   * 根据URL查找菜单项（包括父级菜单）
   */
  private findMenuItemByUrl(url: string): any {
    const findInMenu = (items: any[], parent: any = null): any => {
      for (const item of items) {
        if (item.link === url) {
          return item;
        }
        if (item.children) {
          const found = findInMenu(item.children, item);
          if (found) return found;
        }
      }
      return null;
    };

    return findInMenu(MENU_ITEMS);
  }

  /**
   * 根据URL获取菜单项图标
   */
  private getIconForUrl(url: string): string | undefined {
    const menuItem = this.findMenuItemByUrl(url);
    if (menuItem && menuItem.icon) {
      return menuItem.icon;
    }

    // For child routes, try to find parent icon
    const findParentIcon = (items: any[]): string | undefined => {
      for (const item of items) {
        if (item.children) {
          const childMatch = item.children.some((child: any) => child.link === url);
          if (childMatch && item.icon) {
            return item.icon;
          }
          const childResult = findParentIcon(item.children);
          if (childResult) return childResult;
        }
      }
      return undefined;
    };

    return findParentIcon(MENU_ITEMS);
  }

  /**
   * 生成Tab ID（基于标题，确保每个页面有唯一ID）
   */
  private generateTabId(title: string): string {
    // 使用标题生成固定ID，每个页面名称对应唯一ID
    return 'tab_' + title.replace(/[^a-zA-Z0-9\u4e00-\u9fa5]/g, '_');
  }

  onMenuClick(event: any) {
    if (event.item.title === 'Log out') {
      event.event.preventDefault();
      this.authService.logout();
    }
  }
}

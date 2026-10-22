import { Component, OnInit, OnDestroy } from '@angular/core';
import { Router } from '@angular/router';
import { NbToastrService } from '@nebular/theme';
import { LocalDataSource } from 'angular2-smart-table';
import { Subject } from 'rxjs';
import { takeUntil } from 'rxjs/operators';

import { ResourceGroupService } from '../resource-group.service';
import { ResourceGroup } from '../models/resource-group.model';
import { withTableRow } from '../../../../@core/utils/smart-table';
import { ClusterContextService } from '../../../../@core/data/cluster-context.service';
import { Cluster } from '../../../../@core/data/cluster.service';
import { ConfirmDialogService } from '../../../../@core/services/confirm-dialog.service';

@Component({
  standalone: false,
  selector: 'ngx-resource-groups-list',
  templateUrl: './resource-groups-list.component.html',
  styleUrls: ['./resource-groups-list.component.scss'],
})
export class ResourceGroupsListComponent implements OnInit, OnDestroy {
  private destroy$ = new Subject<void>();

  source: LocalDataSource = new LocalDataSource();
  loading = false;
  activeCluster: Cluster | null = null;

  settings = {
    add: {
      addButtonContent: '<i class="nb-plus"></i>',
      createButtonContent: '<i class="nb-checkmark"></i>',
      cancelButtonContent: '<i class="nb-close"></i>',
    },
    edit: {
      editButtonContent: '<i class="nb-edit"></i>',
      saveButtonContent: '<i class="nb-checkmark"></i>',
      cancelButtonContent: '<i class="nb-close"></i>',
    },
    delete: {
      deleteButtonContent: '<i class="nb-trash"></i>',
      confirmDelete: true,
    },
    actions: {
      columnTitle: 'Actions',
      add: false,
      edit: false,
      delete: false,
      position: 'right',
      custom: [
        {
          name: 'edit',
          title: '<i class="nb-edit" title="Edit"></i>',
        },
        {
          name: 'delete',
          title: '<i class="nb-trash" title="Delete"></i>',
        },
      ],
    },
    columns: {
      name: {
        title: 'Resource group name',
        type: 'string',
        editable: false,
      },
      id: {
        title: 'ID',
        type: 'number',
        editable: false,
        width: '5%',
      },
      cpu_weight: {
        title: 'CPU weight',
        type: 'number',
        editable: false,
        valuePrepareFunction: (value: any) => value || '-',
      },
      exclusive_cpu_cores: {
        title: 'Exclusive CPU cores',
        type: 'number',
        editable: false,
        valuePrepareFunction: (value: any) => value || '-',
      },
      mem_limit: {
        title: 'Memory limit',
        type: 'string',
        editable: false,
        valuePrepareFunction: (value: any) => value || '-',
      },
      concurrency_limit: {
        title: 'Concurrency limit',
        type: 'number',
        editable: false,
        valuePrepareFunction: (value: any) => value || '-',
      },
      classifiers_count: {
        title: 'Classifier count',
        type: 'number',
        editable: false,
        valuePrepareFunction: withTableRow((value: any, row: ResourceGroup) => row.classifiers?.length || 0),
      },
    },
  };

  constructor(
    private resourceGroupService: ResourceGroupService,
    private router: Router,
    private confirmDialog: ConfirmDialogService,
    private toastrService: NbToastrService,
    private clusterContext: ClusterContextService,
  ) {}

  ngOnInit(): void {
    this.clusterContext.activeCluster$
      .pipe(takeUntil(this.destroy$))
      .subscribe((cluster) => {
        this.activeCluster = cluster;
      });
    
    this.loadResourceGroups();
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
  }

  loadResourceGroups(): void {
    this.loading = true;
    this.resourceGroupService
      .getResourceGroups()
      .pipe(takeUntil(this.destroy$))
      .subscribe({
        next: (groups) => {
          this.source.load(groups);
          this.loading = false;
        },
        error: (error) => {
          console.error('Failed to load resource groups:', error);
          this.toastrService.danger('Failed to load resource group list', 'Error');
          this.loading = false;
        },
      });
  }

  onCustomAction(event: any): void {
    const { action, data } = event;

    switch (action) {
      case 'edit':
        this.editResourceGroup(data);
        break;
      case 'delete':
        this.deleteResourceGroup(data);
        break;
    }
  }

  createResourceGroup(): void {
    this.router.navigate(['/pages/cluster-ops/resource-groups/create']);
  }

  editResourceGroup(group: ResourceGroup): void {
    this.router.navigate(['/pages/cluster-ops/resource-groups/edit', group.name]);
  }

  deleteResourceGroup(group: ResourceGroup): void {
    this.confirmDialog.confirmDelete(group.name, 'This action cannot be undone.').subscribe((confirmed) => {
      if (!confirmed) {
        return;
      }
      this.resourceGroupService
        .deleteResourceGroup(group.name)
        .pipe(takeUntil(this.destroy$))
        .subscribe({
          next: () => {
            this.toastrService.success('Resource group deleted successfully', 'Success');
            this.loadResourceGroups();
          },
          error: () => {
            this.toastrService.danger('Failed to delete resource group', 'Error');
          },
        });
    });
  }

  viewUsage(): void {
    this.router.navigate(['/pages/cluster-ops/resource-groups/usage']);
  }

  viewAnalysis(): void {
    this.router.navigate(['/pages/cluster-ops/resource-groups/analysis']);
  }
}
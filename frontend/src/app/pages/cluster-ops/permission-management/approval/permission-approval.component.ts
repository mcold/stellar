import {
  Component,
  OnInit,
  OnDestroy,
  Input,
  Output,
  EventEmitter,
} from '@angular/core';
import { FormBuilder, FormGroup } from '@angular/forms';
import { Subject } from 'rxjs';
import { takeUntil } from 'rxjs/operators';
import { LocalDataSource } from 'angular2-smart-table';
import { NbDialogService, NbToastrService } from '@nebular/theme';
import { PermissionRequestService } from '../../../../@core/data/permission-request.service';
import { PermissionRequestResponse } from '../../../../@core/data/permission-request.model';
import { PermissionApprovalDetailDialogComponent } from './permission-approval-detail-dialog.component';
import { ConfirmationDialogComponent } from '../shared/confirmation-dialog.component';

@Component({
  standalone: false,
  selector: 'ngx-permission-approval',
  templateUrl: './permission-approval.component.html',
  styleUrls: ['./permission-approval.component.scss'],
})
export class PermissionApprovalComponent implements OnInit, OnDestroy {
  @Input() refresh$: Subject<void>;
  @Output() processed = new EventEmitter<void>();

  // State
  pendingRequests: PermissionRequestResponse[] = [];
  filteredRequests: PermissionRequestResponse[] = [];
  requestsLoading = false;
  typeFilter = 'all';

  // ng2-smart-table - 使用标准edit按钮查看详情（与dashboard、audit-logs保持一致）
  approvalSource: LocalDataSource = new LocalDataSource();
  tableSettings = {
    mode: 'external',
    hideSubHeader: true,
    noDataMessage: 'No pending requests',
    actions: {
      columnTitle: 'Actions',
      add: false,
      edit: true,
      delete: false,
      position: 'right',
    },
    edit: {
      editButtonContent: '<i class="nb-search"></i>',  // 使用搜索图标表示查看详情
    },
    pager: {
      display: true,
      perPage: 10,
    },
    columns: {
      id: {
        title: 'ID',
        type: 'number',
        width: '50px',
      },
      request_type: {
        title: 'Type',
        type: 'html',
        sanitizer: { bypassHtml: true },
        width: '90px',
        valuePrepareFunction: (value: string) => {
          const labels: { [key: string]: string } = {
            grant_role: 'Grant Role',
            grant_permission: 'Grant Permission',
            revoke_permission: 'Revoke Permission',
          };
          return `<span class="badge badge-primary">${labels[value] || value}</span>`;
        },
      },
      applicant_name: {
        title: 'Applicant',
        type: 'string',
        width: '80px',
      },
      target: {
        title: 'Target',
        type: 'string',
      },
      reason: {
        title: 'Reason',
        type: 'string',
      },
      created_at: {
        title: 'Requested At',
        type: 'string',
        width: '180px',
        valuePrepareFunction: (value: string) => {
          if (!value) return '-';
          return value.replace('T', ' ').substring(0, 19);
        },
      },
    },
  };

  typeOptions = [
    { label: 'All', value: 'all' },
    { label: 'Grant Role', value: 'grant_role' },
    { label: 'Grant Permission', value: 'grant_permission' },
    { label: 'Revoke Permission', value: 'revoke_permission' },
  ];

  selectedRequest: PermissionRequestResponse | null = null;
  approvalForm: FormGroup;
  approvalInProgress = false;

  private destroy$ = new Subject<void>();

  constructor(
    private fb: FormBuilder,
    private permissionService: PermissionRequestService,
    private dialogService: NbDialogService,
    private toastr: NbToastrService,
  ) {
    this.approvalForm = this.fb.group({
      comment: [''],
    });
  }

  ngOnInit(): void {
    if (this.refresh$) {
      this.refresh$.pipe(takeUntil(this.destroy$)).subscribe(() => {
        this.loadPendingRequests();
      });
    }
    this.loadPendingRequests();
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
  }

  loadPendingRequests(): void {
    this.requestsLoading = true;
    this.permissionService.listPendingApprovals().subscribe({
      next: (requests) => {
        this.pendingRequests = requests;
        this.applyFilters();
        this.requestsLoading = false;
      },
      error: (err) => {
        console.error('Failed to load pending requests:', err);
        this.toastr.danger('Failed to load pending requests', 'Error');
        this.requestsLoading = false;
      },
    });
  }

  onTypeFilterChange(): void {
    this.applyFilters();
  }

  private applyFilters(): void {
    if (this.typeFilter === 'all') {
      this.filteredRequests = [...this.pendingRequests];
    } else {
      this.filteredRequests = this.pendingRequests.filter(
        (req) => req.request_type === this.typeFilter,
      );
    }

    const tableData = this.filteredRequests.map(req => ({
      id: req.id,
      request_type: req.request_type,
      applicant_name: req.applicant_name,
      target: this.getTargetDescription(req),
      reason: req.reason?.substring(0, 50) + (req.reason?.length > 50 ? '...' : ''),
      created_at: req.created_at,
    }));
    this.approvalSource.load(tableData);
  }

  // 使用edit事件处理查看详情
  onEditRow(event: any): void {
    const request = this.filteredRequests.find(r => r.id === event.data.id);
    if (request) {
      this.onViewDetail(request);
    }
  }

  onViewDetail(request: PermissionRequestResponse): void {
    const dialogRef = this.dialogService.open(PermissionApprovalDetailDialogComponent, {
      context: {
        request: request,
        showActions: true,
      },
      hasBackdrop: true,
      closeOnBackdropClick: false,
      closeOnEsc: true,
    });

    dialogRef.onClose.subscribe((result) => {
      if (result) {
        if (result.action === 'approve') {
          this.performApproval(request);
        } else if (result.action === 'reject') {
          this.performRejection(request);
        }
      }
    });
  }


  private performApproval(request: PermissionRequestResponse): void {
    const dialogRef = this.dialogService.open(ConfirmationDialogComponent, {
      context: {
        title: `Approve Request #${request.id}`,
        message: `Do you want to approve the permission request from ${request.applicant_name}?`,
        confirmText: 'Approve',
        cancelText: 'Cancel',
        confirmButtonStatus: 'success',
        confirmIcon: 'checkmark-outline',
        showCommentInput: true,
        commentLabel: 'Approval Comment',
        commentPlaceholder: 'Enter an approval comment (optional)...',
        commentRequired: false,
      },
      hasBackdrop: true,
      closeOnBackdropClick: false,
    });

    dialogRef.onClose.subscribe((result) => {
      if (result && result.confirmed) {
        this.approvalInProgress = true;
        this.permissionService.approveRequest(request.id, { comment: result.comment || '' }).subscribe({
          next: () => {
            this.toastr.success(`Request #${request.id} approved`, 'Approval Successful');
            this.approvalInProgress = false;
            this.processed.emit();
            this.loadPendingRequests();
          },
          error: (err) => {
            console.error('Failed to approve request:', err);
            this.toastr.danger('Failed to approve request: ' + (err.error?.message || err.message), 'Error');
            this.approvalInProgress = false;
          },
        });
      }
    });
  }

  private performRejection(request: PermissionRequestResponse): void {
    const dialogRef = this.dialogService.open(ConfirmationDialogComponent, {
      context: {
        title: `Reject Request #${request.id}`,
        message: `Please explain why the permission request from ${request.applicant_name} is rejected.`,
        confirmText: 'Reject',
        cancelText: 'Cancel',
        confirmButtonStatus: 'danger',
        confirmIcon: 'close-outline',
        showCommentInput: true,
        commentLabel: 'Rejection Reason',
        commentPlaceholder: 'Enter the rejection reason...',
        commentRequired: true,
        commentHint: 'A reason is required when rejecting a request',
      },
      hasBackdrop: true,
      closeOnBackdropClick: false,
    });

    dialogRef.onClose.subscribe((result) => {
      if (result && result.confirmed && result.comment) {
        this.approvalInProgress = true;
        this.permissionService.rejectRequest(request.id, { comment: result.comment }).subscribe({
          next: () => {
            this.toastr.success(`Request #${request.id} rejected`, 'Rejection Successful');
            this.approvalInProgress = false;
            this.processed.emit();
            this.loadPendingRequests();
          },
          error: (err) => {
            console.error('Failed to reject request:', err);
            this.toastr.danger('Failed to reject request: ' + (err.error?.message || err.message), 'Error');
            this.approvalInProgress = false;
          },
        });
      }
    });
  }

  getTargetDescription(request: PermissionRequestResponse): string {
    const details = request.request_details;
    if (!details) return '-';

    if (request.request_type === 'grant_role') {
      return `${details.target_user || '-'} ← ${details.target_role || '-'}`;
    } else {
      const user = details.target_user || '-';
      const resourceType = details.resource_type?.toLowerCase();
      let scope = '';
      if (resourceType === 'catalog') {
        scope = details.catalog || '*';
      } else if (resourceType === 'database') {
        const catalog = details.catalog ? `${details.catalog}.` : '';
        scope = `${catalog}${details.database || '*'}.*`;
      } else if (resourceType === 'table') {
        const catalog = details.catalog ? `${details.catalog}.` : '';
        const db = details.database || '*';
        const table = details.table || '*';
        scope = `${catalog}${db}.${table}`;
      } else {
        scope = details.scope || details.database || details.catalog || '-';
      }
      const perms = details.permissions?.join(', ') || '';
      return perms ? `${user} @ ${scope} (${perms})` : `${user} @ ${scope}`;
    }
  }
}

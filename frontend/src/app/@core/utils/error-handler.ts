import { NbToastrService } from '@nebular/theme';

export class ErrorHandler {
  static extractErrorMessage(error: any): string {
    if (!error) return 'Operation failed, please try again later';

    // Handle string error directly
    if (typeof error === 'string') return error;

    // HttpErrorResponse: error.error contains the response body
    let body = error.error;
    
    // If body is a string, try to parse it as JSON
    if (typeof body === 'string') {
      try {
        body = JSON.parse(body);
      } catch (e) {
        return body; // plain string error message
      }
    }
    
    // Backend returns {code, message} format
    if (body && typeof body === 'object' && body.message) {
      return body.message;
    }
    
    // Nested error structure
    if (body?.error?.message) {
      return body.error.message;
      }
      
    // Direct message on error object (but not Angular's generic Http failure message)
    if (error.message && !error.message.includes('Http failure')) {
        return error.message;
      }
      
    // Status code based message
      if (error.status) {
        return this.getDefaultMessageByStatus(error.status);
    }
    
    return 'Operation failed, please try again later';
  }
  
  /**
   * Handle cluster-related errors and return user-friendly message
   * 处理集群相关错误，统一返回用户友好的错误提示
   */
  static handleClusterError(error: any): string {
    const errorMsg = this.extractErrorMessage(error);
    
    // If error message mentions "No active cluster" or similar, return unified message
    if (errorMsg.includes('No active cluster') ||
        (error.status === 404 && errorMsg.includes('System function'))) {
      return 'Please activate a cluster first';
    }
    
    // If it's a 404, likely a cluster issue
    if (error.status === 404) {
      return 'Please activate a cluster first';
    }
    
    return errorMsg;
  }
  
  private static getDefaultMessageByStatus(status: number): string {
    const statusMessages: { [key: number]: string } = {
      400: 'Invalid request parameters',
      401: 'No permission to perform this operation',
      403: 'No permission to perform this operation',
      404: 'The requested resource does not exist',
      500: 'Internal server error',
      503: 'Service temporarily unavailable'
    };
    return statusMessages[status] || 'Network request failed';
  }

  /**
   * Handle HTTP error and show toast notification
   */
  static handleHttpError(error: any, toastrService: NbToastrService): void {
    const errorMsg = this.extractErrorMessage(error);
    toastrService.danger(errorMsg, 'Error', { duration: 5000 });
  }
}
